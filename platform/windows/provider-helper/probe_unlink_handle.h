#pragma once
#include <windows.h>
#include <cstdint>
#include <cwchar>
#include <limits>

// Capability-only operation: never accepts, resolves or opens a pathname.
// STDIN is the caller's exact inherited DELETE-capable file/directory handle.
// A successful reply means delete-pending; the caller closes its retained
// copies only after validating the reply, completing deletion of that object.
namespace gent_probe_unlink {
enum class Status : std::uint32_t {
    Marked = 0, Arguments = 1, Handle = 2, Type = 3,
    Identity = 4, Bounds = 5, Disposition = 6
};
struct Result { Status status; DWORD error; std::uint32_t kind; };
inline bool decimal(const wchar_t* value, std::uint64_t& result) noexcept {
    if (!value || !*value) return false;
    result = 0;
    unsigned count = 0;
    for (; *value; ++value) {
        if (++count > 20 || *value < L'0' || *value > L'9') return false;
        const auto digit = static_cast<std::uint64_t>(*value - L'0');
        if (result > ((std::numeric_limits<std::uint64_t>::max)() - digit) / 10) return false;
        result = result * 10 + digit;
    }
    return true;
}
inline Result mark(HANDLE handle, std::uint64_t volume, std::uint64_t index,
                   std::uint64_t bytes, std::uint32_t kind) noexcept {
    if (handle == nullptr || handle == INVALID_HANDLE_VALUE || GetFileType(handle) != FILE_TYPE_DISK)
        return {Status::Handle, ERROR_INVALID_HANDLE, kind};
    BY_HANDLE_FILE_INFORMATION info{};
    if (!GetFileInformationByHandle(handle, &info)) return {Status::Handle, GetLastError(), kind};
    const bool directory = (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
    if ((info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0 || info.nNumberOfLinks != 1 ||
        (kind != 1 && kind != 2) || directory != (kind == 2))
        return {Status::Type, ERROR_INVALID_DATA, kind};
    const auto actual_index = (static_cast<std::uint64_t>(info.nFileIndexHigh) << 32) | info.nFileIndexLow;
    const auto actual_bytes = (static_cast<std::uint64_t>(info.nFileSizeHigh) << 32) | info.nFileSizeLow;
    if (volume != info.dwVolumeSerialNumber || index != actual_index || bytes != actual_bytes)
        return {Status::Identity, ERROR_INVALID_DATA, kind};
    if (!directory && bytes > 2u * 1024u * 1024u)
        return {Status::Bounds, ERROR_FILE_TOO_LARGE, kind};
    FILE_DISPOSITION_INFO disposition{};
    disposition.DeleteFile = TRUE;
    // The kernel refuses nonempty directories. No recursion or path fallback.
    if (!SetFileInformationByHandle(handle, FileDispositionInfo, &disposition, sizeof(disposition)))
        return {Status::Disposition, GetLastError(), kind};
    return {Status::Marked, ERROR_SUCCESS, kind};
}
inline int run(int argc, wchar_t** argv) noexcept {
    std::uint64_t volume = 0, index = 0, bytes = 0;
    std::uint32_t kind = 0;
    Result result{Status::Arguments, ERROR_INVALID_PARAMETER, 0};
    if (argc == 6 && std::wcscmp(argv[1], L"host-unlink-probe-handle") == 0 &&
        decimal(argv[2], volume) && decimal(argv[3], index) && decimal(argv[4], bytes)) {
        if (std::wcscmp(argv[5], L"file") == 0) kind = 1;
        if (std::wcscmp(argv[5], L"directory") == 0) kind = 2;
        if (kind != 0) result = mark(GetStdHandle(STD_INPUT_HANDLE), volume, index, bytes, kind);
    }
    // Fixed 20-byte, little-endian Windows ABI; no names or file data emitted.
    struct Reply { char magic[8]; std::uint32_t status, error, kind; };
    static_assert(sizeof(Reply) == 20);
    const Reply reply{{'G','E','N','T','U','N','L','1'}, static_cast<std::uint32_t>(result.status), result.error, result.kind};
    DWORD written = 0;
    if (!WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), &reply, sizeof(reply), &written, nullptr) || written != sizeof(reply)) return 125;
    return result.status == Status::Marked ? 0 : 125;
}
}
