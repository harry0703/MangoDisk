#pragma once

#include <algorithm>

// This protocol is private to two instances of the same embedded DLL. Explorer
// accepts only bounded layout data; it never executes commands or opens a new log.
namespace taskbar {
constexpr ULONG_PTR protocol = 0x4d445801;
inline std::wstring const& channel_class() {
    // A newer app build must never silently reuse an older DLL's implementation.
    // The verified cache basename identifies the exact adapter in both processes.
    static std::wstring name = [] {
        wchar_t path[32768]{};
        DWORD length = GetModuleFileNameW(bridge_module, path, 32768);
        if (!length || length >= 32768) winrt::throw_last_error();
        auto file = wcsrchr(path, L'\\');
        return std::wstring(L"MangoDisk.Taskbar.Xaml.") + (file ? file + 1 : path);
    }();
    return name;
}
constexpr UINT release_message = WM_APP + 1;
enum class Placement : UINT { Right, AfterStart, BarLeft, BarRight };
struct Request {
    UINT version = 2;
    DWORD owner = 0;
    UINT width = 0;
    UINT height = 0;
    UINT gap = 0;
    Placement placement = Placement::Right;
    wchar_t log_file[1024]{};
};

inline void receipt(wchar_t const* path, std::wstring const& event, bool failed = false) {
    HANDLE file = CreateFileW(path, FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE,
                             nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (file == INVALID_HANDLE_VALUE) return;
    SYSTEMTIME now{};
    GetSystemTime(&now);
    wchar_t prefix[100];
    swprintf_s(prefix, L"[%04u-%02u-%02u][%02u:%02u:%02u][taskbar_xaml][%s] ",
               now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond,
               failed ? L"WARN" : L"INFO");
    auto text = winrt::to_string(std::wstring(prefix) + event + L"\n");
    DWORD written = 0;
    WriteFile(file, text.data(), static_cast<DWORD>(text.size()), &written, nullptr);
    CloseHandle(file);
}

inline ux::FrameworkElement find(ux::DependencyObject const& object,
                                 wchar_t const* name, unsigned depth = 0) {
    if (depth > 12) return nullptr;
    auto element = object.try_as<ux::FrameworkElement>();
    if (element && element.Name() == name) return element;
    int count = ux::Media::VisualTreeHelper::GetChildrenCount(object);
    for (int index = 0; index < count; ++index) {
        auto match = find(ux::Media::VisualTreeHelper::GetChild(object, index), name, depth + 1);
        if (match) return match;
    }
    return nullptr;
}

inline bool same_value(double a, double b) {
    // XAML may store these double-valued properties at float precision. At
    // fractional DPI, exact equality mistakes our own margin for an external
    // update and repeatedly adds the reservation. This tolerance is below one
    // physical pixel even at the largest supported scale and width.
    return a == b || std::abs(a - b) < 1.0 / 64.0;
}
inline bool equal(ux::Thickness const& a, ux::Thickness const& b) {
    return same_value(a.Left, b.Left) && same_value(a.Top, b.Top) && same_value(a.Right, b.Right) && same_value(a.Bottom, b.Bottom);
}

// All XAML references and property mutations stay on Explorer's owning UI thread.
// Idle channels keep only a weak root reference and run no timer. The native HWND
// gives a later companion a reusable entry point without another diagnostics pass.
struct Lease {
    winrt::weak_ref<ux::FrameworkElement> root;
    winrt::weak_ref<ux::FrameworkElement> target;
    ux::Thickness original{}, applied{};
    double original_min = 0, applied_min = 0, original_max = 0, applied_max = 0, amount = 0;
    double measured_width = 0, measured_frame_width = 0;
    ULONGLONG measured_at = 0;
    Placement placement = Placement::Right;
    bool active = false;
    HWND host = nullptr, channel = nullptr;
    HANDLE owner = nullptr;
    DWORD owner_id = 0;
    std::wstring log_file;

    explicit Lease(ux::FrameworkElement const& frame, HWND island)
        : root(winrt::make_weak(frame)), host(island) {}
    ~Lease() { restore(); }

    void restore() noexcept {
        // Every release path, including a failed XAML property access, ends the
        // owner watcher. An idle cached channel must not keep polling Explorer.
        if (channel) KillTimer(channel, 1);
        try {
            if (active) {
                auto element = target.get();
                bool margin_owned = element && equal(element.Margin(), applied);
                if (margin_owned) element.Margin(original);
                bool max_owned = element &&
                    (placement == Placement::BarLeft || placement == Placement::BarRight) &&
                    same_value(element.MaxWidth(), applied_max);
                if (max_owned) element.MaxWidth(original_max);
                if (element && placement == Placement::AfterStart && same_value(element.MinWidth(), applied_min)) {
                    element.MinWidth(original_min);
                }
                if (auto frame = root.get()) frame.UpdateLayout();
                receipt(log_file.c_str(), L"resident_taskbar_xaml_restored owned=" +
                        std::to_wstring(margin_owned) + L" max_width_owned=" + std::to_wstring(max_owned) + L" owner=" + std::to_wstring(owner_id));
            }
        } catch (...) {
            receipt(log_file.c_str(), L"resident_taskbar_xaml_restore_failed code=" +
                    std::to_wstring(winrt::to_hresult()), true);
        }
        active = false;
        target = nullptr;
        if (owner) CloseHandle(owner);
        owner = nullptr;
        owner_id = 0;
    }

    HRESULT locate(HWND window, Request const& request, ux::FrameworkElement const& frame) {
        HWND shell = FindWindowW(L"Shell_TrayWnd", nullptr);
        RECT bar{}, tray{};
        if (!GetWindowRect(shell, &bar)) return E_PENDING;
        int x = 0;
        if (request.placement == Placement::BarLeft) {
            x = bar.left + request.gap;
        } else if (request.placement == Placement::AfterStart) {
            auto start = target.get();
            POINT origin{};
            if (!start || !ClientToScreen(host, &origin)) return E_PENDING;
            auto point = start.TransformToVisual(nullptr).TransformPoint({0, 0});
            double scale = frame.XamlRoot().RasterizationScale();
            x = origin.x + static_cast<int>(std::lround((point.X + start.ActualWidth()) * scale))
                + request.gap;
        } else {
            HWND notification = FindWindowExW(shell, nullptr, L"TrayNotifyWnd", nullptr);
            if (!notification || !GetWindowRect(notification, &tray)) return E_PENDING;
            x = tray.left - request.gap - request.width;
        }
        int y = bar.top + (bar.bottom - bar.top - static_cast<int>(request.height)) / 2;
        if (x < bar.left || x + static_cast<int>(request.width) > bar.right || y < bar.top ||
            y + static_cast<int>(request.height) > bar.bottom) return E_PENDING;
        // This window is never shown. Its rectangle transports the actual slot
        // across the private synchronous channel without a second UIA gap search.
        // The helper reads it only after this message acknowledges the layout.
        if (!SetWindowPos(window, nullptr, x, y, request.width, request.height,
                          SWP_NOZORDER | SWP_NOACTIVATE)) return HRESULT_FROM_WIN32(GetLastError());
        return S_OK;
    }

    HRESULT apply(HWND window, Request const& request) {
        if (request.version != 2 || !request.owner || !request.width || request.width > 32768 ||
            !request.height || request.height > 32768 || !request.gap || request.gap > 128 || request.placement > Placement::BarRight ||
            request.log_file[1023] != 0) return E_INVALIDARG;
        auto length = wcsnlen_s(request.log_file, 1024);
        if (length < 13 || wcscmp(request.log_file + length - 13, L"MangoDisk.log") != 0) {
            // sizeof("MangoDisk.log") includes its terminator; the name is 13 characters.
            return E_INVALIDARG;
        }
        if (owner && WaitForSingleObject(owner, 0) != WAIT_TIMEOUT) restore();
        if (owner && owner_id != request.owner) return HRESULT_FROM_WIN32(ERROR_BUSY);
        auto frame = root.get();
        if (!frame) return E_PENDING;
        HWND shell = FindWindowW(L"Shell_TrayWnd", nullptr);
        UINT dpi = GetDpiForWindow(shell);
        if (!dpi) return E_PENDING;
        double wanted = (request.width + 2.0 * request.gap) * 96.0 / dpi;
        auto element = find(frame, request.placement == Placement::AfterStart
            ? L"LaunchListButton" : L"TaskbarFrameRepeater");
        if (!element) return E_NOINTERFACE;
        if (element.ActualWidth() <= 0 || frame.ActualWidth() <= 0) return E_PENDING;
        bool centered = request.placement == Placement::BarLeft || request.placement == Placement::BarRight;
        // Limit capacity without adding space to the group's desired size.
        // Margins (even symmetric ones) can trigger Explorer's narrow-taskbar
        // alignment policy and shift otherwise unconstrained centered buttons.
        double reserved_width = centered ? wanted * 2 : wanted;
        if (reserved_width + 48 >= frame.ActualWidth()) {
            restore();
            return HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER);
        }
        double base_max = active && target.get() == element &&
            (placement == Placement::BarLeft || placement == Placement::BarRight) &&
            same_value(element.MaxWidth(), applied_max) ? original_max : element.MaxWidth();
        double centered_room = frame.ActualWidth() - reserved_width;
        if (request.placement == Placement::BarRight) {
            RECT bar{}, tray{};
            HWND notification = FindWindowExW(shell, nullptr, L"TrayNotifyWnd", nullptr);
            if (!notification || !GetWindowRect(shell, &bar) || !GetWindowRect(notification, &tray)) return E_PENDING;
            centered_room = 2 * ((tray.left - bar.left) * 96.0 / dpi - wanted) - frame.ActualWidth();
        }
        // Measure the natural span without arranging an intermediate layout.
        // In overflow, DesiredSize reports only visible buttons and recycled
        // children remain in the tree, so neither is a usable natural width.
        // Both property changes run synchronously on Explorer's UI thread; only
        // the final constraint is arranged and presented.
        double button_width = element.DesiredSize().Width;
        if (centered && active && target.get() == element &&
            !same_value(applied_max, original_max) && same_value(element.MaxWidth(), applied_max)) {
            // The native window ticks faster than the shell geometry sampler.
            // Do not invalidate Explorer layout on every paint/visibility tick:
            // that competes with UIA snapshots on crowded taskbars. Geometry or
            // placement changes bypass the one-second application-change poll.
            if (GetTickCount64() - measured_at >= 1000 || amount != wanted ||
                placement != request.placement || measured_frame_width != frame.ActualWidth()) {
                element.MaxWidth(original_max);
                element.Measure({static_cast<float>(frame.ActualWidth()), static_cast<float>(frame.ActualHeight())});
                measured_width = element.DesiredSize().Width;
                measured_frame_width = frame.ActualWidth();
                measured_at = GetTickCount64();
                element.MaxWidth(applied_max);
                frame.UpdateLayout();
            }
            button_width = measured_width;
        }
        bool constrain = centered && button_width >= centered_room - 1.0 / 64.0;
        double wanted_max = constrain ? (std::min)(base_max, frame.ActualWidth() - reserved_width) : base_max;
        if (active && placement == request.placement && amount == wanted && target.get() == element &&
            equal(element.Margin(), applied) &&
            (!centered || (same_value(applied_max, wanted_max) && same_value(element.MaxWidth(), applied_max)))) {
            return locate(window, request, frame);
        }

        restore();
        log_file = request.log_file;
        owner = OpenProcess(SYNCHRONIZE, FALSE, request.owner);
        if (!owner) return HRESULT_FROM_WIN32(GetLastError());
        owner_id = request.owner;
        original = element.Margin();
        original_min = element.MinWidth();
        original_max = element.MaxWidth();
        applied_max = wanted_max;
        measured_width = button_width;
        measured_frame_width = frame.ActualWidth();
        measured_at = GetTickCount64();
        applied_min = element.ActualWidth();
        applied = original;
        // Keep the shell's positioning margins unchanged for centered buttons.
        // Only layouts that would collide with the monitor enter overflow.
        if (!centered) applied.Right += wanted;
        amount = wanted;
        placement = request.placement;
        target = winrt::make_weak(element);
        active = true;
        // Start's repeater slot can become narrower than its margin. Preserve its
        // real hit-test width; otherwise UIA reports an empty start-button region
        // and the monitor can cover the still-painted Windows glyph.
        if (placement == Placement::AfterStart) element.MinWidth(applied_min);
        if (centered) element.MaxWidth(applied_max);
        element.Margin(applied);
        // The acknowledgement is a layout barrier, not merely a property-set
        // receipt. UIA must be able to observe the new button bounds afterward.
        frame.UpdateLayout();
        if (!SetTimer(window, 1, 250, nullptr)) {
            DWORD code = GetLastError();
            HRESULT error = HRESULT_FROM_WIN32(code ? code : ERROR_NOT_ENOUGH_MEMORY);
            restore();
            return error;
        }
        receipt(log_file.c_str(), L"resident_taskbar_xaml_reserved edge=" +
                std::wstring(placement == Placement::BarLeft ? L"BarLeft" :
                    placement == Placement::BarRight ? L"BarRight" :
                    placement == Placement::AfterStart ? L"AfterStart" : L"Right") +
                L" centered=" + std::to_wstring(centered) + L" constrained=" + std::to_wstring(constrain) +
                L" buttons_dip=" + std::to_wstring(button_width) +
                L" frame_dip=" + std::to_wstring(frame.ActualWidth()) +
                L" width_dip=" +
                std::to_wstring(wanted) + L" max_width_dip=" + std::to_wstring(centered ? applied_max : original_max) + L" dpi=" + std::to_wstring(dpi) +
                L" owner=" + std::to_wstring(owner_id));
        return locate(window, request, frame);
    }
};

inline LRESULT CALLBACK window_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    auto lease = reinterpret_cast<Lease*>(GetWindowLongPtrW(window, GWLP_USERDATA));
    if (message == WM_NCCREATE) {
        auto create = reinterpret_cast<CREATESTRUCTW*>(lparam);
        // Creation may deliver WM_NCDESTROY before CreateWindowEx returns null.
        // Transfer ownership here, so that failure cannot delete the lease twice
        // (once in this procedure and once in the caller's unique_ptr).
        auto pending = static_cast<std::unique_ptr<Lease>*>(create->lpCreateParams);
        SetLastError(0);
        if (!SetWindowLongPtrW(window, GWLP_USERDATA,
                              reinterpret_cast<LONG_PTR>(pending->get())) && GetLastError()) {
            return FALSE;
        }
        pending->get()->channel = window;
        pending->release();
        return TRUE;
    }
    if (!lease) return DefWindowProcW(window, message, wparam, lparam);
    try {
        if (message == WM_COPYDATA) {
            auto packet = reinterpret_cast<COPYDATASTRUCT*>(lparam);
            if (!packet || packet->dwData != protocol || packet->cbData != sizeof(Request) ||
                !packet->lpData) return E_INVALIDARG;
            if (!lease->root.get()) {
                DestroyWindow(window);
                return E_PENDING;
            }
            return lease->apply(window, *static_cast<Request const*>(packet->lpData));
        }
        if (message == release_message && lease->owner_id == static_cast<DWORD>(wparam)) {
            lease->restore();
            return S_OK;
        }
        if (message == WM_TIMER && lease->owner &&
            WaitForSingleObject(lease->owner, 0) != WAIT_TIMEOUT) {
            lease->restore();
            return 0;
        }
    } catch (...) {
        HRESULT error = winrt::to_hresult();
        lease->restore();
        return error;
    }
    if (message == WM_NCDESTROY) {
        SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        delete lease;
    }
    return DefWindowProcW(window, message, wparam, lparam);
}
} // namespace taskbar
