use std::fmt::Write as _;

use base64::{Engine as _, engine::general_purpose::STANDARD};

use crate::{
    Catalogue, Contact, ExportError, StripResult, invalid,
    package::{Artifact, Detection, Payload},
};

const EMAIL: &str = include_str!("../templates/results-email.eml");
const SMS: &str = include_str!("../templates/results-sms.txt");
const FOOTER_START: &str = "You also can find the test results on the webpage:";
const HTML_PART: &str =
    "Content-Type: text/html; charset=\"UTF-8\"\nContent-Transfer-Encoding: quoted-printable\n\n";
const HTML_EXAMPLE_PREFIX: &str = " We have analyzed your\nsample under the code word ---";
const BOUNDARY: &str = "gydt-results-alternative-1";

pub(crate) fn validate_contact(contact: Contact<'_>) -> Result<Contact<'_>, ExportError> {
    match contact {
        Contact::Email(address) => match address.trim() {
            "" => Ok(Contact::WordPressOnly),
            address if mailbox(address) => Ok(Contact::Email(address)),
            _ => Err(invalid(
                "email address",
                "Enter one email address without a display name.",
            )),
        },
        Contact::Text(number) if number.trim().is_empty() => Ok(Contact::WordPressOnly),
        other => Ok(other),
    }
}

fn mailbox(address: &str) -> bool {
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    address.len() <= 254
        && local.len() <= 64
        && local.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || b"!#$%&'*+-/=?^_`{|}~".contains(&byte)
                })
        })
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

pub(crate) fn make(
    contact: Contact<'_>,
    catalogue: &Catalogue,
    samples: &[Payload<'_>],
    stem: &str,
) -> Result<Option<Artifact>, ExportError> {
    let summarize = || {
        samples
            .iter()
            .enumerate()
            .map(|(index, sample)| sample_summary(index + 1, sample, catalogue))
            .collect::<Result<Vec<_>, _>>()
    };

    let (extension, bytes) = match contact {
        Contact::WordPressOnly => return Ok(None),
        Contact::Text(_) => {
            let body = sample_body(introduction(SMS)?, &summarize()?);
            ("txt", body.into_bytes())
        }
        Contact::Email(address) => {
            let summaries = summarize()?;
            let email = EMAIL.replace("\r\n", "\n");
            let subject = email
                .lines()
                .find_map(|line| line.strip_prefix("Subject: "))
                .ok_or(ExportError::Template)?;
            let (_, footer) = email
                .split_once(FOOTER_START)
                .ok_or(ExportError::Template)?;
            let (footer, _) = footer.split_once("\n--").ok_or(ExportError::Template)?;
            let mut body = sample_body(introduction(&email)?, &summaries);
            body.push('\n');
            body.push_str(FOOTER_START);
            body.push_str(footer.trim_end());
            body.push('\n');
            let html = html_body(&email, &summaries)?;
            (
                "eml",
                email_draft(address, subject, &body, html).into_bytes(),
            )
        }
    };
    Ok(Some(Artifact {
        name: format!("{stem}.{extension}"),
        bytes,
    }))
}

fn introduction(template: &str) -> Result<&str, ExportError> {
    let (_, intro) = template.split_once("Hello.").ok_or(ExportError::Template)?;
    let (greeting, _) = intro
        .split_once("We have analyzed")
        .ok_or(ExportError::Template)?;
    Ok(greeting.trim())
}

fn sample_body(intro: &str, summaries: &[String]) -> String {
    format!("Hello. {intro}\n\n{}", summaries.join("\n"))
}

fn sample_summary(
    number: usize,
    sample: &Payload<'_>,
    catalogue: &Catalogue,
) -> Result<String, ExportError> {
    let mut body = String::new();
    let detected = sample
        .results
        .iter()
        .map(|row| describe(row, catalogue))
        .collect::<Result<Vec<_>, _>>()?;
    let _ = writeln!(
        body,
        "Sample {}\nCode word: {}\nDetected substances: {}",
        number,
        sample.client,
        detected.join(", ")
    );
    let strips = [
        ("Fentanyl", sample.strips.fentanyl),
        ("Benzodiazepines", sample.strips.benzodiazepines),
        ("Medetomidine", sample.strips.medetomidine),
        ("Lysergamide (LSD)", sample.strips.lysergamide),
    ];
    let strips: Vec<_> = strips
        .into_iter()
        .filter_map(|(name, result)| {
            let result = match result {
                StripResult::NotUsed => return None,
                StripResult::Positive => "Positive",
                StripResult::Negative => "Negative",
                StripResult::Invalid => "Invalid",
            };
            Some(format!("{name}: {result}"))
        })
        .collect();
    if !strips.is_empty() {
        let _ = writeln!(body, "Test strips: {}", strips.join(", "));
    }
    if let Some(notes) = &sample.notes {
        let _ = writeln!(body, "Notes: {notes}");
    }
    Ok(body)
}

fn html_body(template: &str, summaries: &[String]) -> Result<String, ExportError> {
    let (_, part) = template
        .split_once(HTML_PART)
        .ok_or(ExportError::Template)?;
    let (encoded, _) = part.split_once("\n--").ok_or(ExportError::Template)?;
    let encoded = encoded.replace('\n', "\r\n");
    let decoded = quoted_printable::decode(encoded, quoted_printable::ParseMode::Strict)
        .map_err(|_| ExportError::Template)?;
    let html = String::from_utf8(decoded)
        .map_err(|_| ExportError::Template)?
        .replace("\r\n", "\n");

    // Replace the template's example introduction and list while preserving its formatted footer.
    let (intro, rest) = html.split_once("<ol>").ok_or(ExportError::Template)?;
    let (greeting, after_example) = html_example(intro)?;
    let (_, footer) = rest.split_once("</ol>").ok_or(ExportError::Template)?;
    let mut result = format!("{greeting}{after_example}<ol>");
    for summary in summaries {
        let escaped = summary
            .trim_end()
            .lines()
            .map(|line| {
                let escaped = line
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                match escaped.strip_prefix("Code word: ") {
                    Some(code) => format!("Code word: {}", code.replace("  ", " &nbsp;")),
                    None => escaped,
                }
            })
            .collect::<Vec<_>>()
            .join("<br>");
        let _ = write!(
            result,
            "<li><font size=\"2\" face=\"arial, sans-serif\">{escaped}</font></li>"
        );
    }
    let _ = write!(result, "</ol>{footer}");
    Ok(result)
}

fn html_example(intro: &str) -> Result<(&str, &str), ExportError> {
    let (greeting, example) = intro
        .split_once(HTML_EXAMPLE_PREFIX)
        .ok_or(ExportError::Template)?;
    let (placeholders, suffix) = example.split_once('.').ok_or(ExportError::Template)?;
    let placeholders = placeholders.trim();
    // Some templates include a labelled four-dash placeholder after the code word.
    let legacy = placeholders
        .strip_suffix("----")
        .map(str::trim)
        .is_some_and(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphabetic() || b.is_ascii_whitespace())
        });
    if placeholders.is_empty() || legacy {
        Ok((greeting, suffix))
    } else {
        Err(ExportError::Template)
    }
}

fn describe(row: &Detection, catalogue: &Catalogue) -> Result<String, ExportError> {
    let label = match row.substance.as_str() {
        "unknown" => "Unknown".into(),
        "other" => row
            .other
            .as_ref()
            .map_or_else(|| "Other".into(), |detail| format!("Other ({detail})")),
        token => {
            let id = token.parse().map_err(|_| ExportError::Catalogue)?;
            catalogue
                .get(id)
                .ok_or(ExportError::Catalogue)?
                .label
                .clone()
        }
    };
    if row.percentage == "blank" {
        Ok(label)
    } else {
        Ok(format!("{label} ({})", row.percentage.replace('-', "–")))
    }
}

fn email_draft(address: &str, subject: &str, text: &str, html: String) -> String {
    let mut draft = format!(
        "To: {address}\r\nSubject: {subject}\r\nX-Unsent: 1\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"{BOUNDARY}\"\r\n\r\n"
    );
    for (part, body) in [("plain", text.replace('\n', "\r\n")), ("html", html)] {
        let _ = write!(
            draft,
            "--{BOUNDARY}\r\nContent-Type: text/{part}; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n"
        );
        let encoded = STANDARD.encode(body);
        for chunk in encoded.as_bytes().chunks(76) {
            // The encoder emits ASCII exclusively.
            draft.extend(chunk.iter().copied().map(char::from));
            draft.push_str("\r\n");
        }
    }
    let _ = write!(draft, "--{BOUNDARY}--\r\n");
    draft
}

#[cfg(test)]
mod tests {
    use super::{HTML_EXAMPLE_PREFIX, HTML_PART, html_body};

    #[test]
    fn html_replaces_both_example_placeholder_shapes() -> Result<(), crate::ExportError> {
        let samples = ["Sample 1\nCode word: SYNTHETIC\nDetected substances: Unknown".into()];
        for placeholders in ["", " legacy reference ---- "] {
            let template = format!(
                "{HTML_PART}<div>Fixture greeting.\n{HTML_EXAMPLE_PREFIX}{placeholders}.\n<p>Keep this.</p>\n<ol><li>Old list.</li></ol><p>Fixture footer.</p></div>\n--fixture--"
            );
            let html = html_body(&template, &samples)?;
            assert!(html.contains("Fixture greeting.") && html.contains("Keep this."));
            assert!(html.contains("Fixture footer.") && html.contains("Code word: SYNTHETIC"));
            assert!(!html.contains("Old list.") && !html.contains("legacy reference"));
            assert!(!html.contains("---") && !html.contains("Submission number"));
        }
        Ok(())
    }
}
