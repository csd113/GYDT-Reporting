// Windows-only camera broker. It owns the stream so the desktop can cancel blocked OS calls.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <fcntl.h>
#include <io.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Devices.Enumeration.h>
#include <winrt/Windows.Graphics.Imaging.h>
#include <winrt/Windows.Media.Capture.h>
#include <winrt/Windows.Media.Capture.Frames.h>
#include <winrt/Windows.Media.Devices.h>
#include <winrt/Windows.Media.MediaProperties.h>
#include <winrt/Windows.Storage.Streams.h>
#include <algorithm>
#include <atomic>
#include <chrono>
#include <memory>
#include <cstdio>
#include <stdexcept>
#include <thread>
#include <vector>

using namespace winrt;
using namespace Windows::Media::Capture;
using namespace Windows::Media::Capture::Frames;
using namespace Windows::Media::MediaProperties;
using namespace Windows::Storage::Streams;
using namespace Windows::Graphics::Imaging;
using namespace std::chrono_literals;

struct Camera {
    MediaFrameSourceGroup group;
    MediaFrameSourceInfo info;
    hstring device;
};

static std::vector<Camera> cameras() {
    std::vector<Camera> result;
    for (auto const& group : MediaFrameSourceGroup::FindAllAsync().get()) {
        for (auto const& info : group.SourceInfos()) {
            if (info.SourceKind() != MediaFrameSourceKind::Color ||
                (info.MediaStreamType() != MediaStreamType::VideoPreview &&
                 info.MediaStreamType() != MediaStreamType::VideoRecord)) continue;
            auto device = info.DeviceInformation();
            if (!device || device.Id().empty()) continue;
            auto current = std::find_if(result.begin(), result.end(), [&](auto const& item) {
                return item.device == device.Id();
            });
            if (current == result.end()) {
                if (result.size() == 32) throw std::runtime_error("too many cameras");
                result.push_back({group, info, device.Id()});
            } else if (current->info.MediaStreamType() != MediaStreamType::VideoPreview &&
                       info.MediaStreamType() == MediaStreamType::VideoPreview) {
                *current = {group, info, device.Id()};
            }
        }
    }
    return result;
}
static void bytes(void const* data, size_t length) {
    if (std::fwrite(data, 1, length, stdout) != length) throw std::runtime_error("closed pipe");
}
static void word(uint32_t value) { bytes(&value, sizeof(value)); }
static void string(hstring const& value) {
    auto utf8 = to_string(value);
    if (utf8.empty() || utf8.size() > 4096) throw std::runtime_error("invalid camera identity");
    word(static_cast<uint32_t>(utf8.size()));
    bytes(utf8.data(), utf8.size());
}
static void list() {
    auto all = cameras();
    word(static_cast<uint32_t>(all.size()));
    for (auto const& camera : all) {
        string(camera.device);
        string(camera.info.DeviceInformation().Name());
    }
}
static void preview(hstring const& id) {
    auto all = cameras();
    auto match = all.begin();
    if (id.empty()) {
        if (all.size() != 1) {
            word(all.empty() ? 1u : 2u);
            return;
        }
    } else {
        match = std::find_if(all.begin(), all.end(), [&](auto const& item) {
            return item.device == id;
        });
        if (match == all.end()) {
            word(3u);
            return;
        }
    }
    MediaCapture capture;
    MediaCaptureInitializationSettings settings;
    settings.SourceGroup(match->group);
    settings.StreamingCaptureMode(StreamingCaptureMode::Video);
    settings.MemoryPreference(MediaCaptureMemoryPreference::Cpu);
    settings.SharingMode(MediaCaptureSharingMode::SharedReadOnly);
    capture.InitializeAsync(settings).get();
    auto source = capture.FrameSources().Lookup(match->info.Id());
    // Keep the negotiated source dimensions and aspect ratio. Only the
    // display texture is reduced; JPEG capture uses these native pixels.
    auto reader = capture.CreateFrameReaderAsync(source, MediaEncodingSubtypes::Bgra8()).get();
    reader.AcquisitionMode(MediaFrameReaderAcquisitionMode::Realtime);
    auto arrival = std::make_shared<std::atomic<uint64_t>>(0);
    auto lost = std::make_shared<std::atomic<bool>>(false);
    auto arrived = reader.FrameArrived(auto_revoke, [arrival](auto const&, auto const&) { ++*arrival; });
    auto failed = capture.Failed(auto_revoke, [lost](auto const&, auto const&) { *lost = true; });
    if (reader.StartAsync().get() != MediaFrameReaderStartStatus::Success) {
        throw std::runtime_error("could not start color video");
    }
    word(0u);
    string(match->device);
    string(match->info.DeviceInformation().Name());
    if (std::fflush(stdout) != 0) throw std::runtime_error("closed pipe");
    // An in-memory packet is produced only when the desktop asks. There is no queue.
    uint64_t issued = 0;
    while (!*lost && std::getchar() == 'f') {
        auto const deadline = std::chrono::steady_clock::now() + 2s;
        MediaFrameReference frame{nullptr};
        do {
            auto const latest = arrival->load();
            if (latest != issued && !*lost) {
                frame = reader.TryAcquireLatestFrame();
                if (frame) issued = latest;
            }
            if (!frame) std::this_thread::sleep_for(20ms);
        } while (!frame && !*lost && std::chrono::steady_clock::now() < deadline);
        if (!frame) throw std::runtime_error("color video stopped");
        auto video = frame.VideoMediaFrame();
        auto picture = video ? video.SoftwareBitmap() : nullptr;
        if (!picture || picture.BitmapPixelFormat() != BitmapPixelFormat::Bgra8)
            throw std::runtime_error("invalid preview bitmap");
        auto const width = picture.PixelWidth(), height = picture.PixelHeight();
        // Match the desktop JPEG and WordPress package bounds before allocation.
        if (width < 1 || width > 4096 || height < 1 || height > 4096 || width * height > 8388608)
            throw std::runtime_error("camera bitmap exceeds the supported JPEG dimensions");
        auto const count = static_cast<uint32_t>(width * height * 4);
        Buffer buffer(count);
        buffer.Length(count);
        picture.CopyToBuffer(buffer);
        if (buffer.Length() != count) throw std::runtime_error("wrong preview size");
        word(static_cast<uint32_t>(width));
        word(static_cast<uint32_t>(height));
        bytes(buffer.data(), count);
        if (std::fflush(stdout) != 0) throw std::runtime_error("closed pipe");
    }
    arrived.revoke();
    failed.revoke();
    reader.StopAsync().get();
    reader.Close();
    capture.Close();
}
int wmain(int argc, wchar_t** argv) {
    if (argc < 3 || argc > 4) return 1;
    try {
        auto owner = OpenProcess(SYNCHRONIZE, FALSE, std::stoul(argv[1]));
        if (!owner) return 1;
        // Ensure a crash before or during native initialization cannot orphan a camera.
        std::thread([owner] {
            WaitForSingleObject(owner, INFINITE);
            std::_Exit(1);
        }).detach();
        _setmode(_fileno(stdin), _O_BINARY);
        _setmode(_fileno(stdout), _O_BINARY);
        init_apartment();
        std::wstring_view mode(argv[2]);
        if (mode == L"list" && argc == 3) list();
        else if (mode == L"preview" && (argc == 3 || argv[3][0] != 0))
            preview(argc == 4 ? hstring(argv[3]) : hstring{});
        else return 1;
        return std::fflush(stdout) == 0 ? 0 : 1;
    } catch (hresult_error const& e) {
        // Status only, no camera identifier, name, or frame bytes.
        return e.code() == E_ACCESSDENIED ? 2 : 1;
    } catch (...) { return 1; }
}

