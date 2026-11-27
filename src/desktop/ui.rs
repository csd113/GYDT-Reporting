use std::{collections::HashMap, hash::Hash, path::Path};

use eframe::egui::{
    self, Align, Button, Color32, Context, FontId, Frame, Id, Image, Key, Layout, Margin,
    Modifiers, Popup, Rect, RectAlign, Response, RichText, ScrollArea, Stroke, TextEdit, Ui,
    UiBuilder, vec2,
};
use gydt_results_compiler::{Catalogue, Profile, live_textures};
use oxiarc_jpeg::{DecodeLimits, DecodeOptions, Decoder, EntropyCoding, PixelFormat};

use super::{
    APP_NAME, App, Completion, Dialog, SettingsPane, Tab, TabOrigin,
    export::{self, Selection as ExportSelection},
    model::{
        COLOURS, Channel, Draft, PhotoCopy, Result, ResultRow, SHAPES, Selection, Site, Strip,
        parse_visit, pill_texture, valid_date,
    },
    store::Settings,
};

mod theme;

pub(super) use theme::configure;
use theme::{
    ACCENT_SUBTLE, BADGE, BORDER, BUTTON, CONTACT, CONTACT_LINE, CONTROL, CONTROL_HEIGHT,
    DEEP_INDIGO, FOCUS, FOOTER_HEIGHT, GUTTER, INDIGO, INK, INSET, INVALID, MUTED, Rule,
    SETTINGS_HEADING_HEIGHT, SURFACE, TABS_HEIGHT, TITLE, TITLE_HEIGHT, TOOLBAR, accent_shape,
    background, band, bold, bold_font, focus_ring,
};

const FORM_VIEWPORT: &str = "sample form viewport";

pub(super) fn orbit() -> Result<egui::IconData> {
    let tree = resvg::usvg::Tree::from_str(
        include_str!("../../assets/app-icon.svg"),
        &resvg::usvg::Options::default(),
    )?;
    let size = 120;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(size, size).ok_or("Can't render the app icon.")?;
    let fitted = resvg::tiny_skia::Transform::from_scale(
        120.0 / tree.size().width(),
        120.0 / tree.size().height(),
    );
    resvg::render(&tree, fitted, &mut pixmap.as_mut());
    let rgba = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let pixel = pixel.demultiply();
            [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
        })
        .collect();
    Ok(egui::IconData {
        rgba,
        width: size,
        height: size,
    })
}

pub(super) fn thumbnail(ctx: &Context, photo: &PhotoCopy) -> Option<egui::TextureHandle> {
    let bytes = photo.bytes().ok()?;
    let display = thumbnail_pixels(&bytes)?;
    let size = [display.width() as usize, display.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, display.as_raw());
    Some(ctx.load_texture("Sample photo", color, egui::TextureOptions::LINEAR))
}

pub(super) fn thumbnail_pixels(bytes: &[u8]) -> Option<image::RgbaImage> {
    let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
        .ok()
        .or_else(|| arithmetic_preview(bytes))?;
    if decoded.width() <= 240 && decoded.height() <= 200 {
        Some(decoded.into_rgba8())
    } else {
        Some(decoded.thumbnail(240, 200).into_rgba8())
    }
}

fn arithmetic_preview(bytes: &[u8]) -> Option<image::DynamicImage> {
    // This fallback only makes display pixels. The original attachment still
    // goes through the existing engine and keeps its original bytes.
    let options = DecodeOptions {
        limits: DecodeLimits {
            max_width: 4096,
            max_height: 4096,
            max_pixels: 8_388_608,
            max_output_bytes: 32 * 1024 * 1024,
            max_input_bytes: super::model::MAX_JPEG,
            ..DecodeLimits::strict()
        },
        ..DecodeOptions::default()
    };
    let mut decoder = Decoder::with_options(bytes, options);
    let info = decoder.read_info().ok()?;
    if info.entropy != EntropyCoding::Arithmetic {
        return None;
    }
    let pixels = decoder.decode().ok()?;
    let (width, height) = (info.scaled_width, info.scaled_height);
    match info.pixel_format() {
        PixelFormat::L8 => {
            image::GrayImage::from_raw(width, height, pixels).map(image::DynamicImage::ImageLuma8)
        }
        PixelFormat::Rgb8 => {
            image::RgbImage::from_raw(width, height, pixels).map(image::DynamicImage::ImageRgb8)
        }
        PixelFormat::Cmyk8 => {
            // The attachment contract excludes Adobe APP14. Use the unprofiled
            // CMYK channels directly for the display conversion.
            let (pixels, []) = pixels.as_chunks::<4>() else {
                return None;
            };
            let rgb = pixels
                .iter()
                .flat_map(|&[c, m, y, k]| {
                    [c, m, y].map(|ink| {
                        let light = u16::from(255 - ink) * u16::from(255 - k);
                        u8::try_from((light + 127) / 255).unwrap_or(255)
                    })
                })
                .collect();
            image::RgbImage::from_raw(width, height, rgb).map(image::DynamicImage::ImageRgb8)
        }
        _ => None,
    }
}

pub(super) struct SubstanceChoice {
    value: Selection,
    label: String,
    official: SearchKeys,
    aliases: Vec<SearchKeys>,
}

impl SubstanceChoice {
    fn new(value: Selection, label: String, official: &str, aliases: &[&str]) -> Self {
        Self {
            value,
            label,
            official: SearchKeys::new(official),
            aliases: aliases.iter().map(|alias| SearchKeys::new(alias)).collect(),
        }
    }

    fn match_rank(&self, query: &SearchKeys) -> Option<u8> {
        if self.official.plain == query.plain {
            Some(0)
        } else if self.official.dash_matches(query, |field, key| field == key) {
            Some(1)
        } else if self.aliases.iter().any(|alias| {
            alias.plain == query.plain || alias.dash_matches(query, |field, key| field == key)
        }) {
            Some(2)
        } else {
            std::iter::once(&self.official)
                .chain(&self.aliases)
                .filter_map(|field| {
                    if field.plain.starts_with(&query.plain)
                        || field.dash_matches(query, |text, key| text.starts_with(key))
                    {
                        Some(3)
                    } else if field.plain.contains(&query.plain)
                        || field.dash_matches(query, |text, key| text.contains(key))
                    {
                        Some(4)
                    } else {
                        None
                    }
                })
                .min()
        }
    }
}

struct SearchKeys {
    plain: String,
    joined: String,
    spaced: String,
}

impl SearchKeys {
    fn new(text: &str) -> Self {
        let plain = text.trim().to_lowercase();
        let joined = normalize_dashes(&plain, true);
        let spaced = normalize_dashes(&plain, false);
        Self {
            plain,
            joined,
            spaced,
        }
    }

    fn dash_matches(&self, query: &Self, matches: impl Fn(&str, &str) -> bool) -> bool {
        (!query.joined.is_empty() && matches(&self.joined, &query.joined))
            || (!query.spaced.is_empty() && matches(&self.spaced, &query.spaced))
    }
}

fn normalize_dashes(text: &str, join: bool) -> String {
    let mut output = String::with_capacity(text.len());
    let (mut pending_space, mut after_dash) = (false, false);
    for c in text.chars() {
        if matches!(
            c,
            '-' | '\u{00ad}' | '\u{2010}'
                ..='\u{2015}' | '\u{2212}' | '\u{fe58}' | '\u{fe63}' | '\u{ff0d}'
        ) {
            pending_space = !join;
            after_dash = true;
        } else if c.is_whitespace() {
            pending_space |= !after_dash;
        } else {
            if pending_space && !output.is_empty() {
                output.push(' ');
            }
            pending_space = false;
            after_dash = false;
            output.push(c);
        }
    }
    output
}

fn search_substances<'a>(options: &'a [SubstanceChoice], query: &str) -> Vec<&'a SubstanceChoice> {
    if query.trim().is_empty() {
        return options.iter().collect();
    }
    let query = SearchKeys::new(query);
    if query.joined.is_empty() {
        return Vec::new();
    }
    let mut matches: Vec<_> = options
        .iter()
        .filter_map(|choice| choice.match_rank(&query).map(|rank| (rank, choice)))
        .collect();
    matches.sort_by_key(|(rank, _)| *rank);
    matches.into_iter().map(|(_, choice)| choice).collect()
}

pub(super) fn substances(catalogue: &Catalogue) -> Vec<SubstanceChoice> {
    let mut labels = HashMap::<&str, usize>::new();
    for record in catalogue.records() {
        *labels.entry(&record.label).or_default() += 1;
    }
    let mut choices: Vec<_> = catalogue
        .records()
        .map(|record| {
            let duplicate = labels
                .get(record.label.as_str())
                .is_some_and(|count| *count > 1);
            let label = if duplicate {
                format!("{} ({}, #{})", record.label, record.slug, record.id)
            } else {
                record.label.clone()
            };
            SubstanceChoice::new(
                Selection::Catalogue(record.id),
                label,
                &record.label,
                &[&record.slug, &record.id.to_string()],
            )
        })
        .collect();
    choices.sort_by_cached_key(|choice| choice.label.to_lowercase());
    choices.insert(
        0,
        SubstanceChoice::new(Selection::Other, "Other".into(), "Other", &[]),
    );
    choices.insert(
        0,
        SubstanceChoice::new(Selection::Unknown, "Unknown".into(), "Unknown", &[]),
    );
    choices
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum SettingsSection {
    Location,
    Export,
}

impl App {
    pub(super) fn show(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        background(ui);
        ui.spacing_mut().item_spacing.y = 0.0;
        if self.dialog.is_some() {
            // The dialog draws the backdrop. Keep the underlying form legible.
            ui.visuals_mut().disabled_alpha = 1.0;
        }
        self.title_bar(ui);
        if !self.show_settings {
            ui.add_enabled_ui(!self.worker.busy() && self.dialog.is_none(), |ui| {
                self.top(ui);
            });
        }
        if self.show_settings
            && let Some(settings) = &mut self.settings
        {
            let action = ui
                .add_enabled_ui(self.dialog.is_none(), |ui| {
                    settings_view(ui, settings, &self.profile)
                })
                .inner;
            match action {
                SettingsAction::ChooseFolder => self.choose_folder(true),
                SettingsAction::ChooseOpusFolder => self.choose_opus_folder(),
                SettingsAction::OpenDataFolder => {
                    if let Err(error) = self.store.open_data_folder() {
                        self.notice("Can't open the data folder", error.to_string());
                    }
                }
                SettingsAction::Save => self.save_settings(),
                SettingsAction::Cancel => self.cancel_settings(),
                SettingsAction::None => {}
            }
        } else if let Some(completion) = &self.completion {
            let action = ui
                .add_enabled_ui(self.dialog.is_none(), |ui| {
                    completion_view(ui, completion, self.save_error.as_deref())
                })
                .inner;
            match action {
                CompletionAction::Back => self.completion = None,
                CompletionAction::Next => self.new_tab(&ctx, false),
                CompletionAction::None => {}
            }
        } else {
            self.reporting(ui, &ctx);
        }
        #[cfg(target_os = "macos")]
        resize_view(&ctx, self.show_settings);
        self.show_dialog(&ctx);
    }

    fn reporting(&mut self, ui: &mut Ui, ctx: &Context) {
        let enabled = self.dialog.is_none() && !self.worker.busy();
        let report = ui.add_enabled_ui(enabled, |ui| self.report_body(ui));
        if let Some((index, edit)) = report.inner {
            if let Some(prior) = edit.prior_site
                && let Err(error) = self.switch_site(index, prior)
            {
                if let Some(tab) = self.tabs.get_mut(index) {
                    tab.draft.site = prior;
                }
                self.notice("Can't change the sample site", error.to_string());
            }
            if edit.date_changed {
                self.rescan();
            }
            if edit.photo {
                if self
                    .tabs
                    .get(index)
                    .is_some_and(|tab| tab.draft.photo.is_some())
                {
                    self.dialog = Some(Dialog::PhotoOptions);
                } else {
                    self.choose_photo(ctx);
                }
            }
        }
        if self.footer(ui, enabled) {
            self.export_active(ctx);
        }
    }

    fn report_body(&mut self, ui: &mut Ui) -> Option<(usize, FormEdit)> {
        let height = (ui.available_height() - FOOTER_HEIGHT).max(64.0);
        let width = ui.available_width();
        let Some((index, tab)) = self
            .active
            .and_then(|index| self.tabs.get_mut(index).map(|tab| (index, tab)))
        else {
            ui.allocate_ui_with_layout(
                vec2(width, height),
                Layout::top_down(Align::Center),
                |ui| {
                    ui.add_space(70.0);
                    ui.heading("Open a sample");
                    ui.label("Use + to start a sample or drop an exported ZIP to reopen it.");
                },
            );
            return None;
        };
        let folder_chosen = self.book.settings.folder.is_some();
        let viewport = Rect::from_min_size(ui.next_widget_position(), vec2(width, height));
        let edit = ui
            .scope_builder(
                UiBuilder::new().max_rect(viewport).ui_stack_info(
                    egui::UiStackInfo::default().with_tag_value(FORM_VIEWPORT, viewport),
                ),
                |ui| {
                    ui.set_min_size(viewport.size());
                    ui.set_clip_rect(ui.clip_rect().intersect(viewport));
                    ScrollArea::vertical()
                        .max_width(width)
                        .id_salt(("sample-form", index))
                        .max_height(height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.push_id(("sample", index), |ui| {
                                sample_form(ui, tab, &self.options, folder_chosen, &self.profile)
                            })
                            .inner
                        })
                        .inner
                },
            )
            .inner;
        Some((index, edit))
    }

    fn top(&mut self, ui: &mut Ui) {
        let (action, gear) = band(
            ui,
            TABS_HEIGHT,
            Color32::TRANSPARENT,
            Rule::Bottom,
            vec2(INSET, 0.0),
            |ui| {
                ui.add_space(12.0);
                let width = ui.available_width();
                ui.allocate_ui_with_layout(
                    vec2(width, 40.0),
                    Layout::left_to_right(Align::Min),
                    |ui| {
                        let tabs = ui
                            .allocate_ui_with_layout(
                                vec2(width - 46.0, 40.0),
                                Layout::left_to_right(Align::Min),
                                |ui| {
                                    ScrollArea::horizontal()
                                        .id_salt("sample-tabs")
                                        .auto_shrink([false, false])
                                        .show(ui, |ui| self.sample_tabs(ui))
                                        .inner
                                },
                            )
                            .inner;
                        let gear = gear_button(ui).clicked();
                        (tabs, gear)
                    },
                )
                .inner
            },
        );
        if gear {
            self.open_settings();
        }
        let ctx = ui.ctx().clone();
        match action {
            TabAction::Select(index) => self.select_tab(&ctx, index),
            TabAction::Close(index) => self.close_tab(&ctx, index),
            TabAction::New(same_visit) => self.new_tab(&ctx, same_visit),
            TabAction::Batch => {
                let selected = (0..self.tabs.len())
                    .map(|index| Some(index) == self.active)
                    .collect();
                self.dialog = Some(Dialog::Batch(selected, false));
            }
            TabAction::None => {}
        }
    }

    fn title_bar(&self, ui: &mut Ui) {
        let inset = if cfg!(target_os = "macos") {
            82.0
        } else {
            INSET
        };
        band(
            ui,
            TITLE_HEIGHT,
            TITLE,
            Rule::Bottom,
            vec2(inset, 12.0),
            |ui| {
                #[cfg(target_os = "macos")]
                {
                    let drag = ui.interact(
                        ui.max_rect(),
                        ui.make_persistent_id("window title"),
                        egui::Sense::drag(),
                    );
                    if drag.drag_started() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                }
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), 32.0),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        ui.add(Image::new(&self.orbit).fit_to_exact_size(vec2(32.0, 32.0)));
                        ui.label(bold(APP_NAME, 20.0));
                    },
                );
            },
        );
    }

    fn sample_tabs(&self, ui: &mut Ui) -> TabAction {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut action = TabAction::None;
        for (index, tab) in self.tabs.iter().enumerate() {
            let selected = self.active == Some(index) && !self.show_settings;
            let show_full = self.tabs.iter().any(|other| {
                other.draft.title != tab.draft.title
                    && short_tab(&other.draft) == short_tab(&tab.draft)
            });
            let text = if show_full {
                tab.draft.tab_label().into()
            } else {
                short_tab(&tab.draft)
            };
            let (select, close) = ui
                .push_id((index, tab.draft.id), |ui| {
                    sample_tab(ui, &text, tab, selected)
                })
                .inner;
            if select {
                action = TabAction::Select(index);
            }
            if close {
                action = TabAction::Close(index);
            }
        }
        ui.scope(|ui| {
            let corner = egui::CornerRadius {
                nw: 8,
                ne: 8,
                sw: 0,
                se: 0,
            };
            let widget = &mut ui.style_mut().visuals.widgets.inactive;
            widget.corner_radius = corner;
            widget.weak_bg_fill = BUTTON;
            widget.bg_stroke = Stroke::new(1.0_f32, BORDER);
            ui.spacing_mut().interact_size.y = 40.0;
            ui.menu_button(bold("+", 19.0), |ui| {
                for (text, enabled, next) in [
                    ("New sample", true, TabAction::New(false)),
                    (
                        "New sample in this visit",
                        super::analysis_day().is_ok_and(|day| self.active_intake(day).is_some()),
                        TabAction::New(true),
                    ),
                ] {
                    if ui.add_enabled(enabled, Button::new(text)).clicked() {
                        action = next;
                        ui.close();
                    }
                }
                ui.separator();
                if self.tabs.len() > 1 && ui.button("Export open samples…").clicked() {
                    action = TabAction::Batch;
                    ui.close();
                }
                hint(ui, "Drop an exported ZIP to reopen it.");
            })
            .response
            .on_hover_text("Sample actions");
        });
        action
    }

    fn footer(&self, ui: &mut Ui, enabled: bool) -> bool {
        band(
            ui,
            FOOTER_HEIGHT,
            TOOLBAR,
            Rule::Top,
            vec2(INSET, 12.0),
            |ui| {
                if self.worker.busy() {
                    export_progress(ui);
                    return false;
                }
                let mut export = false;
                ui.add_enabled_ui(enabled, |ui| {
                    let tab = self.active.and_then(|index| self.tabs.get(index));
                    let width = ui.available_width();
                    column_pair(ui, width - 722.0, 16.0, |ui, left| {
                        if left {
                            if let Some(error) = self.save_error.as_ref().or(self.status.as_ref()) {
                                ui.add(
                                    egui::Label::new(bold(error, 12.0).color(INVALID)).truncate(),
                                )
                                .on_hover_text(error);
                            } else if tab.is_some_and(|tab| tab.missing) {
                                ui.label(bold("Missing data", 12.0).color(INVALID));
                            }
                        } else {
                            export = self.footer_controls(ui, tab);
                        }
                    });
                });
                export
            },
        )
    }

    fn footer_controls(&self, ui: &mut Ui, tab: Option<&Tab>) -> bool {
        let width = ui.available_width();
        ui.allocate_ui_with_layout(
            vec2(width, CONTROL_HEIGHT),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                let format_width = width - 102.0 - 170.0 - 32.0;
                let label = ui.add_sized(
                    [102.0, CONTROL_HEIGHT],
                    egui::Label::new(bold("Export format", 14.0)),
                );
                export_format(ui, format_width).labelled_by(label.id);
                let title = if tab.is_some_and(|tab| self.reexport(tab)) {
                    "Re-export"
                } else {
                    "Export"
                };
                primary_button(ui, title, 170.0, tab.is_some()).clicked()
            },
        )
        .inner
    }

    fn reexport(&self, tab: &Tab) -> bool {
        tab.draft.exported
            || self
                .book
                .history
                .iter()
                .any(|history| history.id == tab.draft.id)
    }
}

#[cfg(target_os = "macos")]
fn resize_view(ctx: &Context, settings: bool) {
    let id = Id::new("reporting window size");
    let previous = ctx.data_mut(|data| {
        let previous = data.get_temp::<bool>(id).unwrap_or_default();
        data.insert_temp(id, settings);
        previous
    });
    if previous != settings {
        let size = if settings {
            super::SETTINGS_SIZE
        } else {
            super::REPORTING_SIZE
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
    }
}

fn short_tab(draft: &Draft) -> String {
    let standard = draft
        .title
        .strip_prefix("3-")
        .and_then(|tail| tail.split_once('-'))
        .filter(|(site, _)| site.len() == 3 && site.bytes().all(|digit| digit.is_ascii_digit()))
        .and_then(|(_, tail)| tail.strip_prefix(&draft.analysis_date))
        .and_then(|tail| tail.strip_prefix('-'));
    if let Some(serial) = standard
        .filter(|suffix| suffix.len() >= 3 && suffix.bytes().all(|digit| digit.is_ascii_digit()))
    {
        format!("Sample {serial}")
    } else {
        draft.tab_label().into()
    }
}

fn sample_tab(ui: &mut Ui, text: &str, tab: &Tab, selected: bool) -> (bool, bool) {
    let shape = ui.painter().add(egui::Shape::Noop);
    let height = 40.0;
    let width = ui
        .fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(text.to_owned(), bold_font(15.0), INK)
                .size()
                .x
        })
        .clamp(74.0, 160.0);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.spacing_mut().button_padding.x = 10.0;
        let foreground = if selected {
            Color32::WHITE
        } else if tab.missing {
            INVALID
        } else {
            INK
        };
        let response = ui.add_sized(
            [width + 16.0, height],
            Button::new(bold(text, 15.0).color(foreground))
                .frame(false)
                .selected(selected)
                .truncate(),
        );
        let response = response.on_hover_text(tab.draft.tab_label());
        let close = ui.add_sized([26.0, height], Button::new("").frame(false));
        let accessible = format!("Close {}", tab.draft.tab_label());
        close.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, accessible.as_str())
        });
        let area = response.rect.union(close.rect);
        let hover = ui.rect_contains_pointer(area);
        let corner = egui::CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        };
        let background = if selected {
            accent_shape(
                area,
                corner,
                ui.is_enabled(),
                response.is_pointer_button_down_on(),
            )
        } else {
            let fill = if hover { ACCENT_SUBTLE } else { BUTTON };
            egui::epaint::RectShape::new(
                area,
                corner,
                fill,
                Stroke::new(1.0_f32, BORDER),
                egui::StrokeKind::Inside,
            )
            .into()
        };
        ui.painter().set(shape, background);
        if hover || close.has_focus() {
            ui.painter().text(
                close.rect.center(),
                egui::Align2::CENTER_CENTER,
                "×",
                FontId::proportional(17.0),
                foreground,
            );
        }
        if response.has_focus() || close.has_focus() {
            let focus = if close.has_focus() {
                close.rect
            } else {
                response.rect
            };
            ui.painter().rect_stroke(
                focus.shrink(3.0),
                4,
                Stroke::new(1.5_f32, if selected { Color32::WHITE } else { FOCUS }),
                egui::StrokeKind::Inside,
            );
        }
        (response.clicked(), close.clicked())
    })
    .inner
}

fn gear_button(ui: &mut Ui) -> Response {
    let response = ui.add_sized([38.0, 34.0], outline_button(""));
    let center = response.rect.center();
    let mut teeth = Vec::new();
    for index in 0_u8..32 {
        let angle = f32::from(index) * std::f32::consts::TAU / 32.0;
        let radius = if index % 4 < 2 { 8.8 } else { 7.0 };
        teeth.push(center + vec2(angle.cos(), angle.sin()) * radius);
    }
    ui.painter().add(egui::Shape::closed_line(
        teeth,
        Stroke::new(1.5_f32, DEEP_INDIGO),
    ));
    ui.painter()
        .circle_stroke(center, 3.2, Stroke::new(1.5_f32, DEEP_INDIGO));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Settings"));
    response.on_hover_text("Settings")
}

fn folder_name(folder: Option<&Path>) -> String {
    folder.map_or_else(
        || "No folder selected".into(),
        |folder| folder.display().to_string(),
    )
}

fn export_format(ui: &mut Ui, width: f32) -> Response {
    combo(
        ui,
        "export-format",
        "WordPress Results ZIP",
        width,
        false,
        |ui| {
            if ui.selectable_label(true, "WordPress Results ZIP").clicked() {
                ui.close();
            }
            ui.add_enabled(false, Button::new("BCCSCU (In Development)").frame(false));
        },
    )
}

fn export_progress(ui: &mut Ui) {
    ui.label(RichText::new("Exporting files…").strong());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), egui::Sense::hover());
    let elapsed = ui.input(|input| input.stable_dt).min(0.07);
    let id = ui.make_persistent_id("export-progress");
    let phase = ui.data_mut(|data| {
        let phase = (data.get_temp::<f32>(id).unwrap_or_default() + elapsed * 1.5) % 2.0;
        data.insert_temp(id, phase);
        phase
    });
    let width = rect.width() * 0.25;
    let left = rect.left() + (rect.width() - width) * (1.0 - (phase - 1.0).abs());
    let indicator =
        egui::Rect::from_min_size(egui::pos2(left, rect.top()), vec2(width, rect.height()));
    ui.painter().rect_filled(rect, 4.0, SURFACE);
    ui.painter().rect_filled(indicator, 4.0, INDIGO);
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(25));
}

enum TabAction {
    None,
    Select(usize),
    Close(usize),
    New(bool),
    Batch,
}

#[derive(Default)]
struct FormEdit {
    date_changed: bool,
    prior_site: Option<Site>,
    photo: bool,
}

fn sample_form(
    ui: &mut Ui,
    tab: &mut Tab,
    options: &[SubstanceChoice],
    folder_chosen: bool,
    profile: &Profile,
) -> FormEdit {
    let before = tab.draft.clone();
    let tab_missing = tab.missing;
    let provisional = tab.origin.is_provisional();
    let mut photo = false;
    let mut outcome = FormEdit::default();
    Frame::new()
        .inner_margin(Margin {
            left: 20,
            right: 35,
            top: 12,
            bottom: 16,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            contact(ui, &mut tab.draft);
            ui.add_space(16.0);
            let columns = ui.available_width() - GUTTER;
            let draw = |ui: &mut Ui, first| {
                if first {
                    left_column(
                        ui,
                        &mut tab.draft,
                        options,
                        tab_missing,
                        folder_chosen,
                        provisional,
                    );
                } else {
                    photo = right_column(
                        ui,
                        &mut tab.draft,
                        tab.thumbnail.as_ref(),
                        tab_missing,
                        profile,
                    );
                }
            };
            if columns < 735.0 {
                let mut draw = draw;
                ui.push_id("left", |ui| draw(ui, true));
                ui.add_space(16.0);
                ui.push_id("right", |ui| draw(ui, false));
            } else {
                column_pair(ui, columns * 0.48, GUTTER, draw);
            }
        });
    if before.texture != tab.draft.texture {
        if tab.draft.texture != "other" {
            tab.draft.texture_other.clear();
        }
        if !pill_texture(&tab.draft.texture) {
            tab.draft.shape.clear();
        }
    }
    if before.shape != tab.draft.shape && tab.draft.shape != "other" {
        tab.draft.shape_other.clear();
    }
    if before.title != tab.draft.title {
        tab.draft.suggested_title = None;
    }
    if before.visit != tab.draft.visit {
        tab.draft.suggested_visit = None;
        if tab.origin == TabOrigin::SameVisit {
            tab.origin = TabOrigin::Proposed;
        }
    }
    if before.site != tab.draft.site {
        outcome.prior_site = Some(before.site);
    }
    outcome.date_changed = before.collection_date != tab.draft.collection_date;
    outcome.photo = photo;
    if before != tab.draft {
        tab.edit();
    }
    outcome
}

fn contact(ui: &mut Ui, draft: &mut Draft) {
    Frame::new()
        .fill(CONTACT)
        .stroke(Stroke::new(1.0_f32, CONTACT_LINE))
        .corner_radius(8)
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                Frame::new()
                    .fill(BADGE)
                    .corner_radius(5)
                    .inner_margin(Margin::symmetric(9, 6))
                    .show(ui, |ui| {
                        ui.label(bold("CLIENT CONTACT", 12.0).color(DEEP_INDIGO));
                    });
                let method = ui.label(bold("Contact method", 14.0));
                let selected = if draft.channel == Channel::None {
                    ""
                } else {
                    draft.channel.label()
                };
                combo(ui, "contact method", selected, 134.0, false, |ui| {
                    for channel in Channel::ALL {
                        select(ui, &mut draft.channel, channel, channel.label());
                    }
                })
                .labelled_by(method.id);
                let address = caption(ui, "Email or phone", "(optional)");
                text(
                    ui,
                    &mut draft.address,
                    "contact address",
                    false,
                    CONTROL_HEIGHT,
                    "",
                )
                .labelled_by(address.id);
            });
        });
}

fn left_column(
    ui: &mut Ui,
    draft: &mut Draft,
    options: &[SubstanceChoice],
    missing: bool,
    folder_chosen: bool,
    provisional: bool,
) {
    ui.spacing_mut().item_spacing.y = 12.0;
    field_group(ui, "Submission number", "(required)", |ui| {
        scoped_error(ui, missing && draft.title.trim().is_empty(), |ui| {
            let width = ui.available_width();
            let font = bold_font(39.0);
            let text_width = ui
                .painter()
                .layout_no_wrap(draft.title.clone(), font, DEEP_INDIGO)
                .size()
                .x;
            let content_width = (width - 24.0).max(1.0);
            let font_size =
                (39.0 * content_width / text_width.max(content_width)).clamp(28.0, 39.0);
            ui.add_sized(
                [width, 54.0],
                TextEdit::singleline(&mut draft.title)
                    .id_salt("submission number")
                    .font(bold_font(font_size))
                    .text_color(DEEP_INDIGO)
                    .background_color(ACCENT_SUBTLE)
                    .margin(Margin::symmetric(11, 5))
                    .vertical_align(Align::Center)
                    .hint_text("Enter submission number"),
            )
        })
    });
    if let Some((zip, client_draft)) = export::preview(draft)
        && (client_draft.is_some()
            || zip.strip_suffix(".zip") != Some(super::model::flat_text(&draft.title).as_str()))
    {
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            for (kind, name) in [("ZIP", Some(zip)), ("Client draft", client_draft)] {
                if let Some(name) = name {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{kind}  {name}"))
                                .size(11.5)
                                .color(MUTED),
                        )
                        .wrap()
                        .selectable(true),
                    );
                }
            }
        });
    }
    if draft.title.is_empty() && !folder_chosen {
        hint(ui, "Set an export folder in Settings or enter a number.");
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let label = caption(ui, "Visit number", "(required)");
        scoped_error(ui, missing && parse_visit(&draft.visit).is_none(), |ui| {
            ui.add_sized(
                [80.0, CONTROL_HEIGHT],
                TextEdit::singleline(&mut draft.visit)
                    .id_salt("visit")
                    .margin(Margin::symmetric(10, 5))
                    .vertical_align(Align::Center),
            )
            .labelled_by(label.id);
        });
        if provisional {
            ui.label(RichText::new("Provisional").small().color(MUTED))
                .on_hover_text("The Visit number is saved with the first sample entry.");
        }
    });
    let expected_missing = missing && draft.expected.is_none();
    field_group(ui, "Expected substance", "(required)", |ui| {
        substance(
            ui,
            "Expected substance",
            &mut draft.expected,
            options,
            ui.available_width(),
            expected_missing,
        )
    });
    if draft.expected == Some(Selection::Other) {
        field(
            ui,
            "Sold As details",
            "(optional)",
            &mut draft.sold_as_other,
            false,
            "",
        );
    }
    physical_details(ui, draft, missing);
    ui.add_space(4.0);
    results(ui, &mut draft.rows, options, missing);
}

fn physical_details(ui: &mut Ui, draft: &mut Draft, missing: bool) {
    string_field(
        ui,
        "Sample texture",
        &mut draft.texture,
        live_textures().iter().map(|value| {
            (
                *value,
                if *value == "other" {
                    "Other (please specify)"
                } else {
                    value
                },
            )
        }),
        "(required)",
        missing,
    );
    if draft.texture == "other" {
        field(
            ui,
            "Texture details",
            "(optional)",
            &mut draft.texture_other,
            false,
            "",
        );
    }
    if pill_texture(&draft.texture) {
        string_field(
            ui,
            "Pill Shape",
            &mut draft.shape,
            SHAPES.iter().copied(),
            "(optional)",
            false,
        );
        if draft.shape == "other" {
            field(
                ui,
                "Shape details",
                "(optional)",
                &mut draft.shape_other,
                false,
                "",
            );
        }
    }
    string_field(
        ui,
        "Sample colour",
        &mut draft.colour,
        COLOURS.iter().map(|value| {
            (
                *value,
                if *value == "other" {
                    "Other (please specify)"
                } else {
                    value
                },
            )
        }),
        "(required)",
        missing,
    );
    if draft.colour == "other" {
        field(
            ui,
            "Colour details",
            "(optional)",
            &mut draft.colour_other,
            false,
            "",
        );
    }
}

fn right_column(
    ui: &mut Ui,
    draft: &mut Draft,
    thumbnail: Option<&egui::TextureHandle>,
    missing: bool,
    profile: &Profile,
) -> bool {
    ui.spacing_mut().item_spacing.y = 12.0;
    let width = ui.available_width();
    let mut photo = false;
    let attachment = draft.photo.as_ref();
    split(
        ui,
        width - 166.0,
        16.0,
        |ui| {
            field(ui, "Code word", "", &mut draft.code_word, false, "");
            let date_missing = missing && valid_date(&draft.collection_date).is_none();
            field(
                ui,
                "Collection date",
                "(required)",
                &mut draft.collection_date,
                date_missing,
                "YYYY-MM-DD",
            );
        },
        |ui| {
            photo = photo_target(ui, attachment, thumbnail);
        },
    );
    site_field(
        ui,
        "Collection location",
        "(required)",
        &mut draft.site,
        profile,
    );
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.add_space(8.0);
        strips(ui, &mut draft.strips);
    });
    use_and_effects(ui, draft);
    field_group(ui, "Sample notes", "(optional)", |ui| {
        ui.add_sized(
            [ui.available_width(), 64.0],
            TextEdit::multiline(&mut draft.notes)
                .id_salt("notes")
                .desired_rows(2)
                .margin(Margin::symmetric(10, 6))
                .hint_text(""),
        )
    });
    photo
}

fn photo_target(
    ui: &mut Ui,
    attachment: Option<&PhotoCopy>,
    thumbnail: Option<&egui::TextureHandle>,
) -> bool {
    let size = vec2(ui.available_width(), 132.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let response = response.on_hover_text(attachment.map_or("Attach a JPEG", |photo| &photo.name));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            if attachment.is_some() {
                "Sample photo attached"
            } else {
                "Attach sample photo"
            },
        )
    });
    let fill = if response.hovered() {
        ACCENT_SUBTLE
    } else {
        Color32::from_rgb(226, 233, 245)
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 8, fill);
    let line = Color32::from_rgb(131, 149, 195);
    if response.has_focus() {
        painter.rect_stroke(
            rect.expand(1.5),
            8,
            Stroke::new(1.75_f32, FOCUS),
            egui::StrokeKind::Inside,
        );
    } else {
        let r = rect.shrink(0.5);
        let path = [
            r.left_top() + vec2(8.0, 0.0),
            r.right_top() - vec2(8.0, 0.0),
            r.right_top() + vec2(0.0, 8.0),
            r.right_bottom() - vec2(0.0, 8.0),
            r.right_bottom() - vec2(8.0, 0.0),
            r.left_bottom() + vec2(8.0, 0.0),
            r.left_bottom() - vec2(0.0, 8.0),
            r.left_top() + vec2(0.0, 8.0),
            r.left_top() + vec2(8.0, 0.0),
        ];
        painter.extend(egui::Shape::dashed_line(
            &path,
            Stroke::new(1.0_f32, line),
            3.0,
            2.0,
        ));
    }
    if let Some(image) = thumbnail {
        let area = rect.shrink(7.0);
        let ratio = (area.width() / image.size_vec2().x).min(area.height() / image.size_vec2().y);
        let fit = egui::Rect::from_center_size(area.center(), image.size_vec2() * ratio);
        ui.painter().image(
            image.id(),
            fit,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        draw_empty_photo(painter, rect, attachment.is_some());
    }
    response.clicked()
}

fn draw_empty_photo(painter: &egui::Painter, rect: egui::Rect, attached: bool) {
    let origin = rect.center() - vec2(0.0, 20.0);
    let icon = egui::Rect::from_center_size(origin, vec2(34.0, 28.0));
    let icon_color = Color32::from_rgb(83, 99, 135);
    painter.rect_stroke(
        icon,
        3.0,
        Stroke::new(1.6_f32, icon_color),
        egui::StrokeKind::Inside,
    );
    painter.circle_stroke(
        icon.center() + vec2(6.0, -4.0),
        2.5,
        Stroke::new(1.3_f32, icon_color),
    );
    painter.add(egui::Shape::line(
        vec![
            icon.left_bottom() + vec2(3.0, -4.0),
            icon.left_bottom() + vec2(10.0, -12.0),
            icon.left_bottom() + vec2(17.0, -5.5),
            icon.left_bottom() + vec2(21.0, -8.0),
            icon.right_bottom() + vec2(-2.0, -4.0),
        ],
        Stroke::new(1.5_f32, icon_color),
    ));
    painter.text(
        rect.center() + vec2(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        "Sample photo",
        bold_font(14.0),
        MUTED,
    );
    painter.text(
        rect.center() + vec2(0.0, 29.0),
        egui::Align2::CENTER_CENTER,
        if attached {
            "JPEG attached"
        } else {
            "(no image)"
        },
        FontId::proportional(13.0),
        MUTED,
    );
}

fn results(ui: &mut Ui, rows: &mut Vec<ResultRow>, options: &[SubstanceChoice], missing: bool) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let available = ui.available_width() - 120.0;
        let percentage = (available / 2.55).max(100.0);
        let substance = available - percentage;
        result_headings(ui, substance, percentage);
        let focus_key = ui.make_persistent_id("result row to focus");
        let focus = ui.data_mut(|data| data.remove_temp::<usize>(focus_key));
        let highlight = missing && !rows.iter().any(|row| row.substance.is_some());
        let removable = rows.len() > 1;
        let mut remove = None;
        for (index, row) in rows.iter_mut().enumerate() {
            let controls = RowControls {
                number: index + 1,
                substance,
                percentage,
                highlight: highlight && index == 0,
                focus: focus == Some(index),
                removable,
            };
            if ui
                .push_id(("row", index), |ui| result_row(ui, row, options, controls))
                .inner
            {
                remove = Some(index);
            }
        }
        if let Some(index) = remove {
            rows.remove(index);
            Popup::close_all(ui.ctx());
            ui.data_mut(|data| data.insert_temp(focus_key, index.min(rows.len() - 1)));
            ui.ctx().request_repaint();
        }
        if ui
            .add_enabled(
                rows.len() < 10,
                outline_button("+ Add substance").fill(Color32::from_rgb(237, 243, 255)),
            )
            .clicked()
        {
            ui.data_mut(|data| data.insert_temp(focus_key, rows.len()));
            rows.push(ResultRow::default());
            ui.ctx().request_repaint();
        }
        hint(ui, "Exports in the order shown.");
    });
}

#[derive(Clone, Copy)]
struct RowControls {
    number: usize,
    substance: f32,
    percentage: f32,
    highlight: bool,
    focus: bool,
    removable: bool,
}

fn result_row(
    ui: &mut Ui,
    row: &mut ResultRow,
    options: &[SubstanceChoice],
    controls: RowControls,
) -> bool {
    let remove = ui
        .horizontal(|ui| {
            let (badge, _) = ui.allocate_exact_size(vec2(32.0, 28.0), egui::Sense::hover());
            ui.painter()
                .rect_filled(badge, 4, Color32::from_rgb(220, 231, 248));
            ui.painter().text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                format!("{:02}", controls.number),
                bold_font(12.0),
                Color32::from_rgb(48, 49, 156),
            );
            let field = format!("Detected substance {}", controls.number);
            let dropdown = substance(
                ui,
                &field,
                &mut row.substance,
                options,
                controls.substance,
                controls.highlight,
            );
            if controls.focus {
                dropdown.request_focus();
            }
            range_picker(ui, row, controls.percentage);
            if !controls.removable {
                return false;
            }
            let response = ui.add_sized([28.0, 28.0], outline_button("×").small());
            let label = format!("Remove substance {}", controls.number);
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), &label)
            });
            response.on_hover_text(label).clicked()
        })
        .inner;
    if row.substance == Some(Selection::Other) {
        field(
            ui,
            "Detected substance details",
            "(optional)",
            &mut row.other,
            false,
            "",
        );
    }
    if row.custom {
        field(
            ui,
            "Custom percentage",
            "",
            &mut row.range,
            false,
            "e.g. 10-17%",
        );
    }
    remove
}

fn result_headings(ui: &mut Ui, substance: f32, percentage: f32) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 41.0),
        Layout::left_to_right(Align::Min),
        |ui| {
            ui.add_space(40.0);
            ui.allocate_ui_with_layout(vec2(substance, 41.0), Layout::top_down(Align::Min), |ui| {
                ui.set_min_width(substance);
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.label(bold("Detected substances", 14.0));
                hint(ui, "At least one required");
            });
            ui.allocate_ui_with_layout(
                vec2(percentage, 22.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(bold("Percentage", 14.0));
                    hint(ui, "(optional)");
                },
            );
        },
    );
}

fn range_picker(ui: &mut Ui, row: &mut ResultRow, width: f32) {
    let selected = if row.custom {
        "Custom…".to_owned()
    } else {
        row.range.clone()
    };
    combo(ui, "percentage", &selected, width, false, |ui| {
        if ui
            .selectable_label(!row.custom && row.range.is_empty(), "Blank")
            .clicked()
        {
            row.range.clear();
            row.custom = false;
            ui.close();
        }
        for low in (5..100).step_by(5) {
            let preset = format!("{low}-{}%", low + 5);
            if ui
                .selectable_label(!row.custom && row.range == preset, &preset)
                .clicked()
            {
                row.range = preset;
                row.custom = false;
                ui.close();
            }
        }
        if ui.selectable_label(row.custom, "Custom…").clicked() {
            row.custom = true;
            ui.close();
        }
    });
}

#[derive(Clone, Default)]
struct Search {
    query: String,
    index: usize,
    last_frame: Option<u64>,
}

fn substance(
    ui: &mut Ui,
    name: &str,
    value: &mut Option<Selection>,
    options: &[SubstanceChoice],
    width: f32,
    error: bool,
) -> Response {
    let selected = options
        .iter()
        .find(|choice| Some(choice.value) == *value)
        .map_or("", |choice| choice.label.as_str());
    let search_id = ui.make_persistent_id((name, "search"));
    let response = combo_control(ui, name, selected, width, error, |ui, button| {
        substance_popup(ui, button, |ui, height| {
            substance_menu(ui, search_id, value, options, height);
        });
    });
    response.on_hover_text(name)
}

fn substance_menu(
    ui: &mut Ui,
    search_id: Id,
    value: &mut Option<Selection>,
    options: &[SubstanceChoice],
    height: f32,
) {
    let frame = ui.ctx().cumulative_frame_nr();
    let mut search = ui
        .data_mut(|data| data.get_temp::<Search>(search_id))
        .unwrap_or_default();
    let opened = search
        .last_frame
        .is_none_or(|last| last.saturating_add(1) < frame);
    if opened {
        search = Search::default();
    }
    search.last_frame = Some(frame);
    let down = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowDown));
    let up = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::ArrowUp));
    let enter = ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
    let top = ui.cursor().top();
    // TextEdit requests cursor scrolling on edits. This popup is drawn inside
    // the sample form's scroll pass, so consume its request here instead of
    // letting it scroll the form and move the anchor out of view.
    let field = ScrollArea::neither()
        .id_salt((search_id, "search-field"))
        .show(ui, |ui| {
            ui.add(
                TextEdit::singleline(&mut search.query)
                    .id(search_id)
                    .hint_text("Search substances"),
            )
        })
        .inner;
    if opened {
        field.request_focus();
    }
    let reset = opened || field.changed();
    if reset {
        search.index = 0;
    }
    let query = search.query.trim();
    let choices: Vec<_> = std::iter::once((None, "Blank"))
        .filter(|_| query.is_empty())
        .chain(
            search_substances(options, query)
                .into_iter()
                .map(|choice| (Some(choice.value), choice.label.as_str())),
        )
        .collect();
    if choices.is_empty() {
        hint(ui, "No matching substances.");
    }
    if down {
        search.index = (search.index + 1).min(choices.len().saturating_sub(1));
    }
    if up {
        search.index = search.index.saturating_sub(1);
    }
    if enter
        && !opened
        && let Some((choice, _)) = choices.get(search.index)
    {
        *value = *choice;
        ui.close();
    }
    let keyboard = down || up || !query.is_empty();
    let row_height = ui.spacing().interact_size.y.min(26.0);
    let spacing = ui.spacing().item_spacing.y.min(3.0);
    let row_stride = row_height + spacing;
    // Reserve actual space for the input and a one-line overflow cue. A
    // conservative fixed deduction cost a full row in the smallest window.
    let chrome = ui.cursor().top() - top
        + ui.text_style_height(&egui::TextStyle::Small)
        + ui.spacing().item_spacing.y;
    let list_height = (height - chrome).max(row_stride);
    let visible_rows = (1_u16..=6)
        .rev()
        .find(|&rows| row_stride * f32::from(rows) <= list_height)
        .unwrap_or(1);
    if choices.len() > usize::from(visible_rows) {
        hint(ui, &format!("{} results · scroll", choices.len()));
    }
    let mut scroll = ScrollArea::vertical()
        .id_salt((search_id, "results"))
        .max_height(row_stride * f32::from(visible_rows));
    if reset {
        scroll = scroll.vertical_scroll_offset(0.0);
    }
    scroll.show(ui, |ui| {
        ui.spacing_mut().interact_size.y = row_height;
        ui.spacing_mut().item_spacing.y = spacing;
        for (index, (choice, label)) in choices.iter().enumerate() {
            let active = keyboard && search.index == index;
            let response = ui.add_sized(
                [ui.available_width(), row_height],
                egui::Button::selectable(active || *value == *choice, *label).truncate(),
            );
            if active && (down || up) {
                response.scroll_to_me(None);
            }
            if response.clicked() {
                *value = *choice;
                ui.close();
            }
        }
    });
    ui.data_mut(|data| data.insert_temp(search_id, search));
}

fn strips(ui: &mut Ui, values: &mut [Strip; 4]) {
    ui.label(bold("Test strip results", 16.0));
    let [fentanyl, benzodiazepines, medetomidine, lysergamide] = values;
    let first = (ui.available_width() - 16.0) / 2.0;
    split(
        ui,
        first,
        16.0,
        |ui| {
            strip(ui, "Fentanyl", fentanyl);
            strip(ui, "Medetomidine", medetomidine);
        },
        |ui| {
            strip(ui, "Benzodiazepines", benzodiazepines);
            strip(ui, "Lysergamide (LSD)", lysergamide);
        },
    );
}

fn strip(ui: &mut Ui, name: &str, current: &mut Strip) {
    field_group(ui, name, "", |ui| {
        combo(
            ui,
            name,
            current.label(),
            ui.available_width(),
            false,
            |ui| {
                for result in Strip::ALL {
                    let label = if result == Strip::Blank {
                        "Blank (Not Used)"
                    } else {
                        result.label()
                    };
                    select(ui, current, result, label);
                }
            },
        )
    });
}

fn use_and_effects(ui: &mut Ui, draft: &mut Draft) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        let first = (ui.available_width() - 16.0) / 2.0;
        split(
            ui,
            first,
            16.0,
            |ui| {
                answer(ui, "Has this sample been used?", &mut draft.used);
            },
            |ui| {
                answer(ui, "Unexpected effects?", &mut draft.effects);
            },
        );
        hint(ui, "Select the same answer again to clear it.");
        if draft.effects == Some(true) {
            field(
                ui,
                "Describe the unexpected effects",
                "(optional)",
                &mut draft.effects_other,
                false,
                "",
            );
        }
    });
}

fn answer(ui: &mut Ui, question: &str, answer: &mut Option<bool>) {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let label = caption(ui, question, "");
        ui.spacing_mut().interact_size.y = 30.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (choice, text) in [(true, "Yes"), (false, "No")] {
                let chosen = *answer == Some(choice);
                let color = if chosen { Color32::WHITE } else { INK };
                let button = Button::new(bold(text, 14.0).color(color))
                    .selected(chosen)
                    .corner_radius(15)
                    .fill(if chosen { DEEP_INDIGO } else { SURFACE })
                    .stroke(Stroke::new(1.0_f32, CONTROL));
                if ui
                    .add_sized([52.0, 30.0], button)
                    .labelled_by(label.id)
                    .clicked()
                {
                    *answer = if chosen { None } else { Some(choice) };
                }
            }
        });
    });
}

fn field_group(
    ui: &mut Ui,
    title: &str,
    detail: &str,
    control: impl FnOnce(&mut Ui) -> Response,
) -> Response {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let label = caption(ui, title, detail);
        control(ui).labelled_by(label.id)
    })
    .inner
}

fn field(
    ui: &mut Ui,
    name: &str,
    detail: &str,
    value: &mut String,
    error: bool,
    placeholder: &str,
) -> Response {
    field_group(ui, name, detail, |ui| {
        text(ui, value, name, error, CONTROL_HEIGHT, placeholder)
    })
}

fn text(
    ui: &mut Ui,
    value: &mut String,
    id: impl Hash,
    error: bool,
    height: f32,
    placeholder: &str,
) -> Response {
    scoped_error(ui, error, |ui| {
        ui.add_sized(
            [ui.available_width(), height],
            TextEdit::singleline(value)
                .id_salt(id)
                .margin(Margin::symmetric(10, 5))
                .vertical_align(Align::Center)
                .hint_text(placeholder),
        )
    })
}

fn site_field(
    ui: &mut Ui,
    name: &str,
    detail: &str,
    current: &mut Site,
    profile: &Profile,
) -> Response {
    field_group(ui, name, detail, |ui| {
        combo(
            ui,
            name,
            profile.site((*current).into()).label(),
            ui.available_width(),
            false,
            |ui| {
                for site in Site::ALL {
                    select(ui, current, site, profile.site(site.into()).label());
                }
            },
        )
    })
}

fn string_field<'a>(
    ui: &mut Ui,
    name: &str,
    current: &mut String,
    choices: impl Iterator<Item = (&'a str, &'a str)>,
    detail: &str,
    missing: bool,
) {
    let choices: Vec<_> = choices.collect();
    let selected = choices
        .iter()
        .find(|(value, _)| *value == current)
        .map_or("", |(_, label)| *label);
    field_group(ui, name, detail, |ui| {
        combo(
            ui,
            name,
            selected,
            ui.available_width(),
            missing && current.is_empty(),
            |ui| {
                if ui.selectable_label(current.is_empty(), "Blank").clicked() {
                    current.clear();
                    ui.close();
                }
                for (value, label) in &choices {
                    if ui.selectable_label(current == value, *label).clicked() {
                        (*value).clone_into(current);
                        ui.close();
                    }
                }
            },
        )
    });
}

fn combo(
    ui: &mut Ui,
    id: impl Hash,
    current: &str,
    width: f32,
    error: bool,
    menu: impl FnOnce(&mut Ui),
) -> Response {
    combo_control(ui, id, current, width, error, |ui, button| {
        combo_popup(ui, button, menu);
    })
}

fn combo_control(
    ui: &mut Ui,
    id: impl Hash,
    current: &str,
    width: f32,
    error: bool,
    popup: impl FnOnce(&Ui, &Response),
) -> Response {
    scoped_error(ui, error, |ui| {
        let response = ui
            .push_id(id, |ui| {
                ui.add_sized(
                    [width, CONTROL_HEIGHT],
                    Button::new(current).right_text("   ").truncate(),
                )
            })
            .inner;
        response.widget_info(|| {
            let mut info = egui::WidgetInfo::new(egui::WidgetType::ComboBox);
            info.enabled = response.enabled();
            info.current_text_value = Some(current.to_owned());
            info
        });
        let center = response.rect.right_center() - vec2(16.0, 0.0);
        ui.painter().add(egui::Shape::line(
            vec![
                center + vec2(-3.0, -1.5),
                center + vec2(0.0, 2.0),
                center + vec2(3.0, -1.5),
            ],
            Stroke::new(1.3_f32, INK),
        ));
        focus_ring(ui, &response, 6);
        popup(ui, &response);
        response
    })
}

fn substance_popup(ui: &Ui, button: &Response, menu: impl FnOnce(&mut Ui, f32)) {
    let mut popup = Popup::menu(button)
        .width(button.rect.width())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    let form_viewport = ui
        .stack()
        .iter()
        .find_map(|frame| frame.tags().get_downcast::<Rect>(FORM_VIEWPORT).copied());
    if !button.enabled()
        || form_viewport.is_some_and(|viewport| !viewport.contains_rect(button.rect))
    {
        Popup::close_id(ui.ctx(), popup.get_id());
        return;
    }
    // The popup can extend past the scrollable form, but not past the app window.
    let window = ui.ctx().content_rect();
    let above = button.rect.top() - window.top();
    let below = window.bottom() - button.rect.bottom();
    let opens_below = below >= 226.0 || below >= above;
    let height = if opens_below { below } else { above } - 20.0;
    popup = popup
        .align(if opens_below {
            RectAlign::BOTTOM_START
        } else {
            RectAlign::TOP_START
        })
        .align_alternatives(&[])
        .gap(3.0)
        .frame(Frame::popup(ui.style()).shadow(egui::Shadow::NONE));
    popup.show(|ui| {
        ui.set_min_width(ui.available_width());
        // An upward popup starts with its previous height. Give the list the full
        // bounded budget so it can grow again after a short search.
        ui.set_max_height(height.max(0.0));
        menu(ui, height);
    });
}

fn combo_popup(ui: &Ui, button: &Response, menu: impl FnOnce(&mut Ui)) {
    let viewport = ui
        .stack()
        .iter()
        .find_map(|frame| frame.tags().get_downcast::<Rect>(FORM_VIEWPORT).copied());
    let mut popup = Popup::menu(button)
        .width(button.rect.width())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    let mut height = ui.spacing().combo_height;
    if let Some(viewport) = viewport {
        if !button.enabled() || !viewport.contains_rect(button.rect) {
            Popup::close_id(ui.ctx(), popup.get_id());
            return;
        }
        // Leave space for the menu padding, border, and gap inside the form.
        let above = button.rect.top() - viewport.top() - 20.0;
        let below = viewport.bottom() - button.rect.bottom() - 20.0;
        let opens_below = below >= height || below >= above;
        height = height.min(if opens_below { below } else { above });
        popup = popup
            .align(if opens_below {
                RectAlign::BOTTOM_START
            } else {
                RectAlign::TOP_START
            })
            .align_alternatives(&[])
            .gap(3.0)
            .frame(Frame::popup(ui.style()).shadow(egui::Shadow::NONE));
    }
    popup.show(|ui| {
        if let Some(viewport) = viewport {
            ui.set_clip_rect(ui.clip_rect().intersect(viewport));
        }
        ui.set_min_width(ui.available_width());
        ScrollArea::vertical().max_height(height).show(ui, menu);
    });
}

fn select<T: PartialEq>(ui: &mut Ui, current: &mut T, value: T, label: &str) {
    if ui.selectable_label(*current == value, label).clicked() {
        *current = value;
        ui.close();
    }
}

fn caption(ui: &mut Ui, title: &str, detail: &str) -> Response {
    ui.allocate_ui_with_layout(
        vec2(0.0, 20.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let label = ui.label(bold(title, 14.0));
            if !detail.is_empty() {
                hint(ui, detail);
            }
            label
        },
    )
    .inner
}

fn hint(ui: &mut Ui, value: &str) {
    ui.label(RichText::new(value).small().color(MUTED));
}

fn scoped_error<R>(ui: &mut Ui, show: bool, body: impl FnOnce(&mut Ui) -> R) -> R {
    if !show {
        return body(ui);
    }
    ui.scope(|ui| {
        let visuals = ui.visuals_mut();
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.bg_stroke = Stroke::new(1.8_f32, INVALID);
        }
        visuals.selection.stroke = Stroke::new(1.8_f32, INVALID);
        body(ui)
    })
    .inner
}

fn split(
    ui: &mut Ui,
    first: f32,
    gap: f32,
    left: impl FnOnce(&mut Ui),
    right: impl FnOnce(&mut Ui),
) {
    let (mut left, mut right) = (Some(left), Some(right));
    column_pair(ui, first, gap, |ui, is_left| {
        if is_left {
            if let Some(draw) = left.take() {
                draw(ui);
            }
        } else if let Some(draw) = right.take() {
            draw(ui);
        }
    });
}

fn column_pair(ui: &mut Ui, first: f32, gap: f32, mut draw: impl FnMut(&mut Ui, bool)) {
    let bounds = ui.available_rect_before_wrap();
    let split = bounds.left() + first;
    let mut bottom = bounds.top();
    for (start, end, left) in [
        (bounds.left(), split, true),
        (split + gap, bounds.right(), false),
    ] {
        let size = egui::Rect::from_min_max(
            egui::pos2(start, bounds.top()),
            egui::pos2(end, bounds.bottom()),
        );
        let mut column = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("column", left))
                .max_rect(size)
                .layout(Layout::top_down(Align::Min)),
        );
        column.set_width(size.width());
        column.set_clip_rect(column.clip_rect().intersect(size.with_max_y(f32::INFINITY)));
        draw(&mut column, left);
        bottom = bottom.max(column.min_rect().bottom());
    }
    let used = egui::Rect::from_min_max(bounds.min, egui::pos2(bounds.right(), bottom));
    ui.allocate_rect(used, egui::Sense::hover());
}

fn outline_button(label: &str) -> Button<'_> {
    Button::new(bold(label, 14.0))
        .fill(BUTTON)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(7)
}

fn primary_button(ui: &mut Ui, text: &str, width: f32, enabled: bool) -> Response {
    let draw = ui.painter().add(egui::Shape::Noop);
    let button = Button::new(bold(text, 16.0).color(Color32::WHITE))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .min_size(vec2(width, CONTROL_HEIGHT));
    let response = ui.add_enabled(enabled, button);
    let shape = accent_shape(
        response.rect,
        egui::CornerRadius::same(7),
        response.enabled(),
        response.is_pointer_button_down_on(),
    );
    ui.painter().set(draw, shape);
    focus_ring(ui, &response, 8);
    response
}

enum SettingsAction {
    None,
    ChooseFolder,
    ChooseOpusFolder,
    OpenDataFolder,
    Cancel,
    Save,
}
enum CompletionAction {
    None,
    Back,
    Next,
}

fn settings_view(ui: &mut Ui, settings: &mut SettingsPane, profile: &Profile) -> SettingsAction {
    band(
        ui,
        SETTINGS_HEADING_HEIGHT,
        Color32::TRANSPARENT,
        Rule::Bottom,
        vec2(INSET, 0.0),
        |ui| {
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), SETTINGS_HEADING_HEIGHT),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.label(bold("Settings", 20.0));
                },
            );
        },
    );
    let width = ui.available_width();
    let height = (ui.available_height() - FOOTER_HEIGHT).max(50.0);
    let mut action = SettingsAction::None;
    let mut section = None;
    ui.allocate_ui(vec2(width, height), |ui| {
        ui.set_min_height(height);
        column_pair(ui, 164.0, 0.0, |ui, sidebar| {
            if sidebar {
                section = settings_sidebar(ui, height, settings.section);
            } else {
                ScrollArea::vertical()
                    .id_salt("settings content")
                    .max_height(height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        action = settings_sections(ui, settings, section, profile);
                    });
            }
        });
    });
    band(
        ui,
        FOOTER_HEIGHT,
        TOOLBAR,
        Rule::Top,
        vec2(INSET, 12.0),
        |ui| {
            let width = ui.available_width();
            column_pair(ui, width - 210.0, 0.0, |ui, left| {
                if left {
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), CONTROL_HEIGHT),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 14.0;
                            hint(ui, concat!("Version ", env!("CARGO_PKG_VERSION")));
                            let open = ui.add(outline_button("Open Data Folder")).on_hover_text(
                                "Show local settings, numbering records, and export history.",
                            );
                            focus_ring(ui, &open, 8);
                            if open.clicked() {
                                action = SettingsAction::OpenDataFolder;
                            }
                        },
                    );
                } else {
                    ui.allocate_ui_with_layout(
                        vec2(210.0, CONTROL_HEIGHT),
                        Layout::right_to_left(Align::Center),
                        |ui| {
                            if primary_button(ui, "Save", 104.0, true).clicked() {
                                action = SettingsAction::Save;
                            }
                            if ui.add(outline_button("Cancel")).clicked() {
                                action = SettingsAction::Cancel;
                            }
                        },
                    );
                }
            });
        },
    );
    action
}

fn settings_sidebar(ui: &mut Ui, height: f32, active: SettingsSection) -> Option<SettingsSection> {
    let area = egui::Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), height));
    ui.painter().rect_filled(area, 0, TOOLBAR);
    ui.painter().line_segment(
        [
            area.right_top() - vec2(0.5, 0.0),
            area.right_bottom() - vec2(0.5, 0.0),
        ],
        Stroke::new(1.0_f32, BORDER),
    );
    Frame::new()
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.spacing_mut().interact_size.y = 33.0;
            for (section, label) in [
                (SettingsSection::Location, "Location"),
                (SettingsSection::Export, "Export"),
            ] {
                let selected = active == section;
                let color = if selected { DEEP_INDIGO } else { INK };
                let fill = if selected {
                    ACCENT_SUBTLE
                } else {
                    Color32::TRANSPARENT
                };
                let item = Button::new("")
                    .left_text(bold(label, 14.0).color(color))
                    .selected(selected)
                    .fill(fill)
                    .stroke(Stroke::NONE)
                    .corner_radius(5);
                if ui.add_sized([ui.available_width(), 33.0], item).clicked() {
                    return Some(section);
                }
            }
            None
        })
        .inner
}

fn settings_sections(
    ui: &mut Ui,
    settings: &mut SettingsPane,
    section: Option<SettingsSection>,
    profile: &Profile,
) -> SettingsAction {
    Frame::new()
        .inner_margin(Margin {
            left: 24,
            right: 24,
            top: 16,
            bottom: 20,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width().min(656.0));
            ui.spacing_mut().item_spacing.y = 12.0;
            let location = ui.label(bold("Location", 16.0));
            settings_location(ui, settings, profile);
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(2.0);
            let export = ui.label(bold("Export", 16.0));
            let choose = settings_folder(ui, "Default export folder", settings.draft.folder.as_deref());
            ui.checkbox(&mut settings.draft.attach_opus, "Attach OPUS spectrum to exports");
            let choose_opus = settings_folder(ui, "OPUS spectrum folder", settings.draft.opus_folder.as_deref());
            if settings.draft.attach_opus {
                hint(ui, "The current WordPress importer (v1.0.2) does not accept ZIPs that include OPUS files.");
            }
            if let Some(error) = &settings.error {
                ui.label(RichText::new(error).color(INVALID));
            }
            if let Some(selected) = section {
                settings.section = selected;
                match selected {
                    SettingsSection::Location => location.scroll_to_me(Some(Align::Min)),
                    SettingsSection::Export => export.scroll_to_me(Some(Align::Min)),
                }
            }
            if choose {
                SettingsAction::ChooseFolder
            } else if choose_opus {
                SettingsAction::ChooseOpusFolder
            } else {
                SettingsAction::None
            }
        })
        .inner
}

fn settings_location(ui: &mut Ui, settings: &mut SettingsPane, profile: &Profile) {
    ui.allocate_ui(vec2(360.0, 0.0), |ui| {
        ui.set_width(360.0);
        site_field(
            ui,
            "Default collection location",
            "",
            &mut settings.draft.site,
            profile,
        );
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(bold("Submission codes", 14.0));
            hint(ui, "Enter three digits for each location.");
            ui.add_space(6.0);
            let [east, west] = &mut settings.draft.codes;
            let show_errors = settings.error.is_some();
            split(
                ui,
                172.0,
                16.0,
                |ui| {
                    field(
                        ui,
                        profile.site(Site::ExampleEast.into()).label(),
                        "",
                        east,
                        show_errors && !Settings::valid_code(east),
                        "",
                    );
                },
                |ui| {
                    field(
                        ui,
                        profile.site(Site::ExampleWest.into()).label(),
                        "",
                        west,
                        show_errors && !Settings::valid_code(west),
                        "",
                    );
                },
            );
        });
    });
}

fn settings_folder(ui: &mut Ui, title: &str, folder: Option<&Path>) -> bool {
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        let label = caption(ui, title, "");
        let mut path = folder_name(folder);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let path_width = (ui.available_width() - 148.0).clamp(110.0, 360.0);
            ui.add_sized(
                [path_width, CONTROL_HEIGHT],
                TextEdit::singleline(&mut path)
                    .margin(Margin::symmetric(10, 5))
                    .interactive(false)
                    .vertical_align(Align::Center),
            )
            .labelled_by(label.id)
            .on_hover_text(&path);
            ui.add(outline_button("Choose folder…")).clicked()
        })
        .inner
    })
    .inner
}

fn completion_view(
    ui: &mut Ui,
    completion: &Completion,
    save_error: Option<&str>,
) -> CompletionAction {
    let mut action = CompletionAction::None;
    ui.spacing_mut().item_spacing.y = 12.0;
    ui.add_space(37.0);
    let folder = completion.folder.file_name().map_or_else(
        || completion.folder.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(format!("Data exported to {folder}"))
                .size(26.0)
                .strong()
                .color(DEEP_INDIGO),
        );
        ui.label(completion.folder.display().to_string());
        ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
            for file in &completion.files {
                if let Some(name) = file.file_name() {
                    ui.label(name.to_string_lossy());
                }
            }
        });
        for warning in [completion.warning.as_deref(), save_error]
            .into_iter()
            .flatten()
        {
            ui.label(RichText::new(warning).color(INVALID));
        }
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 197.0).max(0.0) * 0.5);
            if ui.add(outline_button("Go back")).clicked() {
                action = CompletionAction::Back;
            }
            if primary_button(ui, "Next sample", 112.0, true).clicked() {
                action = CompletionAction::Next;
            }
        });
    });
    action
}

enum ModalAction {
    Keep,
    Dismiss,
    Folder,
    Photo,
    Replace,
    Remove,
    Repeat,
    Overwrite,
    Export,
}

impl App {
    fn show_dialog(&mut self, ctx: &Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let key = dialog_key(&dialog);
        let response = egui::Modal::new(Id::new(key))
            .backdrop_color(Color32::from_rgba_unmultiplied(24, 35, 62, 85))
            .frame(
                Frame::new()
                    .fill(SURFACE)
                    .corner_radius(12)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .inner_margin(Margin::symmetric(20, 18)),
            )
            .show(ctx, |ui| {
                ui.set_width(428.0);
                ui.spacing_mut().item_spacing.y = 10.0;
                let height = (ctx.content_rect().height() - 55.0).max(100.0);
                if height < 380.0 {
                    ScrollArea::vertical()
                        .max_height(height)
                        .show(ui, |ui| self.dialog_contents(ui, &mut dialog))
                        .inner
                } else {
                    self.dialog_contents(ui, &mut dialog)
                }
            });
        if response.should_close() {
            return;
        }
        match response.inner {
            ModalAction::Keep => self.dialog = Some(dialog),
            ModalAction::Dismiss => {}
            ModalAction::Folder => self.choose_folder(false),
            ModalAction::Photo => self.choose_photo(ctx),
            ModalAction::Replace => {
                if let Dialog::ReplacePhoto(index, photo) = dialog {
                    self.finish_photo(ctx, index, photo);
                }
            }
            ModalAction::Remove => {
                if let Some(index) = self.active {
                    self.set_photo(ctx, index, None);
                }
            }
            ModalAction::Repeat => {
                if let Dialog::Repeat(mut selection, _) = dialog {
                    selection.confirmed = true;
                    self.begin_export(ctx, selection);
                }
            }
            ModalAction::Overwrite => {
                if let Dialog::Overwrite(selection, plan) = dialog {
                    self.write_plan(ctx, selection, plan);
                }
            }
            ModalAction::Export => {
                if let Dialog::Batch(selected, wordpress_only) = dialog {
                    let tabs = selected
                        .into_iter()
                        .enumerate()
                        .filter_map(|(index, selected)| selected.then_some(index))
                        .collect();
                    self.begin_export(
                        ctx,
                        ExportSelection {
                            tabs,
                            wordpress_only,
                            confirmed: false,
                        },
                    );
                }
            }
        }
    }

    fn dialog_contents(&self, ui: &mut Ui, dialog: &mut Dialog) -> ModalAction {
        match dialog {
            Dialog::ExportFolder => {
                modal_title(ui, "Export folder unavailable");
                ui.label("Choose an existing folder for this export. Your sample will stay open.");
                dialog_buttons(ui, "Cancel", "Choose folder…", ModalAction::Folder)
            }
            Dialog::PhotoPrompt => {
                modal_title(ui, "Sample photo");
                ui.label("Take a photo of the sample before crushing it, then attach the .jpg or .jpeg here.");
                dialog_buttons(ui, "Close", "Attach photo…", ModalAction::Photo)
            }
            Dialog::ReplacePhoto(_, photo) => {
                modal_title(ui, "Replace sample photo?");
                ui.label(format!("Use {} for this sample?", photo.copy.name));
                dialog_buttons(ui, "Cancel", "Replace photo", ModalAction::Replace)
            }
            Dialog::PhotoOptions => {
                modal_title(ui, "Sample photo");
                let current = self
                    .active
                    .and_then(|index| self.tabs.get(index))
                    .and_then(|tab| tab.draft.photo.as_ref());
                if let Some(photo) = current {
                    ui.label(&photo.name);
                }
                ui.horizontal(|ui| {
                    if ui.button("Remove photo").clicked() {
                        return ModalAction::Remove;
                    }
                    dialog_buttons(ui, "Close", "Choose another…", ModalAction::Photo)
                })
                .inner
            }
            Dialog::Batch(selected, wordpress_only) => {
                self.batch_picker(ui, selected, wordpress_only)
            }
            Dialog::Repeat(_, names) => {
                modal_title(ui, "Re-export");
                ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
                    for title in names {
                        ui.label(format!("Sample number {title} has already been used."));
                    }
                });
                dialog_buttons(ui, "Cancel", "Re-export", ModalAction::Repeat)
            }
            Dialog::Overwrite(_, plan) => {
                modal_title(ui, "Export files already exist");
                path_label(ui, plan.folder());
                ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
                    for filename in plan.approved.filenames() {
                        ui.label(filename);
                    }
                });
                dialog_buttons(ui, "Cancel", "Overwrite", ModalAction::Overwrite)
            }
            Dialog::Notice(title, error) => {
                modal_title(ui, title);
                ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                    ui.label(error.as_str());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if default_button(ui, "Close").clicked() {
                        ModalAction::Dismiss
                    } else {
                        ModalAction::Keep
                    }
                })
                .inner
            }
        }
    }

    fn batch_picker(
        &self,
        ui: &mut Ui,
        selected: &mut [bool],
        wordpress_only: &mut bool,
    ) -> ModalAction {
        modal_title(ui, "Select samples to export");
        ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
            for (chosen, tab) in selected.iter_mut().zip(&self.tabs) {
                ui.checkbox(
                    chosen,
                    format!("{}  ·  Visit {}", tab.draft.tab_label(), tab.draft.visit),
                );
            }
        });
        ui.checkbox(wordpress_only, "WordPress only");
        let drafts = selected
            .iter()
            .zip(&self.tabs)
            .filter_map(|(chosen, tab)| chosen.then_some(&tab.draft));
        let first = drafts.clone().next();
        let recipient = if *wordpress_only {
            Ok(None)
        } else {
            export::batch_recipient(drafts)
        };
        match (first, recipient) {
            (Some(first), Ok(Some((channel, address)))) => {
                if let Some(ext) = channel.extension(address) {
                    let stem = super::model::filename_stem(&super::model::flat_text(&first.title));
                    hint(ui, &format!("Client draft: {stem}.{ext}"));
                }
            }
            (_, Err(message)) => {
                ui.label(RichText::new(message).color(INVALID));
            }
            _ => {}
        }
        let repeat = selected.iter().zip(&self.tabs).any(|(chosen, tab)| {
            *chosen
                && (tab.draft.exported
                    || self
                        .book
                        .history
                        .iter()
                        .any(|history| history.id == tab.draft.id))
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if primary_button(
                ui,
                if repeat { "Re-export" } else { "Export" },
                104.0,
                first.is_some() && recipient.is_ok(),
            )
            .clicked()
            {
                return ModalAction::Export;
            }
            if default_button(ui, "Cancel").clicked() {
                return ModalAction::Dismiss;
            }
            ModalAction::Keep
        })
        .inner
    }
}

fn dialog_key(dialog: &Dialog) -> &'static str {
    match dialog {
        Dialog::ExportFolder => "missing export folder",
        Dialog::PhotoPrompt => "photo reminder",
        Dialog::ReplacePhoto(..) => "replace photo",
        Dialog::PhotoOptions => "photo menu",
        Dialog::Batch(..) => "sample selection",
        Dialog::Repeat(..) => "re-export warning",
        Dialog::Overwrite(..) => "overwrite warning",
        Dialog::Notice(..) => "error notice",
    }
}

fn dialog_buttons(ui: &mut Ui, cancel: &str, accept: &str, action: ModalAction) -> ModalAction {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if primary_button(ui, accept, 104.0, true).clicked() {
            return action;
        }
        if default_button(ui, cancel).clicked() {
            return ModalAction::Dismiss;
        }
        ModalAction::Keep
    })
    .inner
}

fn default_button(ui: &mut Ui, text: &str) -> Response {
    let response = ui.button(text);
    if ui.memory(|memory| memory.focused().is_none()) {
        response.request_focus();
    }
    response
}

fn modal_title(ui: &mut Ui, title: &str) {
    ui.label(bold(title, 20.0));
}

fn path_label(ui: &mut Ui, path: &Path) {
    ui.label(path.display().to_string());
}

#[cfg(test)]
mod substance_search_tests {
    use super::{Profile, Selection, SubstanceChoice, search_substances, substances};

    fn choice(id: u64, label: &str, alias: &str) -> SubstanceChoice {
        SubstanceChoice::new(
            Selection::Catalogue(id),
            label.into(),
            label,
            &alias.split_whitespace().collect::<Vec<_>>(),
        )
    }

    #[test]
    fn dash_variants_match_in_both_directions_without_merging_choices() {
        let options = [
            choice(1, "Test-Alpha", "Test-Alpha sample-slug"),
            choice(2, "TestAlpha", "TestAlpha other-slug"),
            choice(3, "Different", "Different"),
        ];
        for query in ["testalpha", "TEST-ALPHA", "Test – ALPHA"] {
            let labels: Vec<_> = search_substances(&options, query)
                .into_iter()
                .map(|found| found.label.as_str())
                .collect();
            assert_eq!(labels.len(), 2, "{query}");
            assert!(labels.contains(&"TestAlpha") && labels.contains(&"Test-Alpha"));
        }
        let spaced = search_substances(&options, "test alpha");
        assert_eq!(spaced.len(), 1);
        assert!(
            spaced
                .first()
                .is_some_and(|hit| hit.value == Selection::Catalogue(1))
        );
        let hits = search_substances(&options, "TestAlpha");
        assert!(
            hits.first()
                .is_some_and(|hit| hit.value == Selection::Catalogue(2))
        );
        assert!(
            hits.get(1)
                .is_some_and(|hit| hit.value == Selection::Catalogue(1))
        );
        assert!(
            search_substances(&options, "sampleslug")
                .first()
                .is_some_and(|hit| hit.value == Selection::Catalogue(1))
        );
        assert!(search_substances(&options, "test-beta").is_empty());
        for separators in ["---", " – — ", "− ‐ －"] {
            assert!(
                search_substances(&options, separators).is_empty(),
                "{separators}"
            );
        }
    }

    #[test]
    fn exact_name_ranks_before_broader_plain_match() {
        let options = [
            choice(1, "Other Test Alpha", "Other Test Alpha"),
            choice(2, "Test Alpha", "Test Alpha"),
        ];
        let hits = search_substances(&options, "test alpha");
        assert_eq!(hits.len(), 2);
        assert!(
            hits.first()
                .is_some_and(|hit| hit.value == Selection::Catalogue(2))
        );
        assert!(
            hits.get(1)
                .is_some_and(|hit| hit.value == Selection::Catalogue(1))
        );
    }
    #[test]
    fn bundled_catalogue_prioritizes_real_names_and_prefixes() -> super::Result<()> {
        let profile = Profile::embedded()?;
        let options = substances(profile.catalogue());
        for (query, expected) in [
            ("fEnTaNyL", "Fentanyl"),
            ("4mmc", "4-MMC"),
            ("4-MMC", "4-MMC"),
            ("Ketamine", "Ketamine"),
        ] {
            let first = search_substances(&options, query);
            assert_eq!(first.first().map(|hit| hit.label.as_str()), Some(expected));
            let again = search_substances(&options, query);
            assert!(
                first
                    .iter()
                    .map(|c| c.value)
                    .eq(again.iter().map(|c| c.value))
            );
        }
        let hits = search_substances(&options, "fenta");
        let before = hits.iter().position(|hit| hit.label == "Fentanyl");
        let after = hits.iter().position(|hit| hit.label == "Furanylfentanyl");
        assert!(matches!((before, after), (Some(prefix), Some(substring)) if prefix < substring));
        Ok(())
    }

    #[test]
    fn exact_aliases_beat_prefixes_but_never_the_official_name() {
        let options = [
            choice(1, "Example wider", "example-wider"),
            choice(2, "Other title", "example"),
            choice(3, "Example", "example-main"),
            choice(4, "Example also", "example-also"),
        ];
        let ids: Vec<_> = search_substances(&options, "EXAMPLE")
            .iter()
            .map(|c| c.value)
            .collect();
        assert_eq!(ids, [3, 2, 1, 4].map(Selection::Catalogue));
    }

    #[test]
    fn pasting_in_search_does_not_scroll_its_form() {
        use eframe::egui::{Area, Context, Event, Id, RawInput, ScrollArea, pos2};

        let ctx = Context::default();
        let search_id = Id::new("picker-regression-search");
        let options = [
            choice(1, "Ketamine", "ketamine"),
            choice(2, "Fentanyl", "fentanyl"),
        ];
        let mut value = None;
        let mut offset = 0.0;
        for (frame, events) in [vec![], vec![Event::Paste("Fentanyl".into())], vec![]]
            .into_iter()
            .enumerate()
        {
            let _ = ctx.run_ui(
                RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.add_space(180.0);
                    let mut form = ScrollArea::vertical().max_height(100.0).animated(false);
                    if frame == 0 {
                        form = form.vertical_scroll_offset(160.0);
                    }
                    offset = form
                        .show(ui, |ui| {
                            ui.add_space(480.0);
                            Area::new(Id::new("opposite-side-popup"))
                                .fixed_pos(pos2(8.0, 16.0))
                                .show(ui.ctx(), |ui| {
                                    ui.set_width(200.0);
                                    super::substance_menu(
                                        ui, search_id, &mut value, &options, 130.0,
                                    );
                                });
                        })
                        .state
                        .offset
                        .y;
                },
            );
        }
        assert!(
            ctx.data_mut(|data| data.get_temp::<super::Search>(search_id))
                .is_some_and(|s| s.query == "Fentanyl")
        );
        assert!((offset - 160.0).abs() < 0.5, "form moved to {offset}");
    }
}
