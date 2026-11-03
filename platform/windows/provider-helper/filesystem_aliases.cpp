#include "filesystem_aliases.h"
#include "win32_paths.h"
#include <stdexcept>

using gent_windows_paths::comparison_path;
using gent_windows_paths::io_path;
static bool within(const std::wstring& path, const std::wstring& root) {
    return path.size() >= root.size() && _wcsnicmp(path.c_str(), root.c_str(), root.size()) == 0 && (path.size() == root.size() || path[root.size()] == L'\\');
}
static bool covered(const std::wstring& path, const std::vector<std::wstring>& roots) {
    for (const auto& root : roots) if (within(path, root)) return true; return false;
}
static void aliases(const std::wstring& path, bool write, bool explicit_read, const std::vector<std::wstring>& readable, const std::vector<std::wstring>& writable) {
    HANDLE file = CreateFileW(io_path(path).c_str(), FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
    if (file == INVALID_HANDLE_VALUE) throw std::runtime_error("alias-file-query:" + std::to_string(GetLastError()));
    BY_HANDLE_FILE_INFORMATION info{}; BOOL queried = GetFileInformationByHandle(file, &info); CloseHandle(file);
    if (!queried) throw std::runtime_error("alias-file-information:" + std::to_string(GetLastError()));
    if (info.nNumberOfLinks <= 1 || (explicit_read && !write)) return;
    if (path.size() < 3 || path[1] != L':' || path[2] != L'\\') throw std::runtime_error("alias-volume-form");
    DWORD length = 32768; std::wstring link(length, L'\0'); HANDLE search = FindFirstFileNameW(io_path(path).c_str(), 0, &length, link.data());
    if (search == INVALID_HANDLE_VALUE) throw std::runtime_error("alias-enumeration:" + std::to_string(GetLastError()));
    bool safe = true; DWORD error = ERROR_SUCCESS;
    do {
        std::wstring alias = path.substr(0, 2) + std::wstring(link.c_str());
        if (!covered(alias, write ? writable : readable)) { safe = false; break; }
        length = static_cast<DWORD>(link.size());
    } while (FindNextFileNameW(search, &length, link.data()));
    if (safe) error = GetLastError(); FindClose(search);
    if (!safe) throw std::runtime_error("filesystem-hardlink-escape");
    if (error != ERROR_HANDLE_EOF) throw std::runtime_error("alias-enumeration:" + std::to_string(error));
}
void validate_filesystem_aliases(const std::vector<std::wstring>& input_read, const std::vector<std::wstring>& input_write) {
    std::vector<std::wstring> read, write;
    for (const auto& root : input_read) read.push_back(comparison_path(root)); for (const auto& root : input_write) { write.push_back(comparison_path(root)); read.push_back(comparison_path(root)); }
    size_t count = 0;
    for (const auto& root : read) {
        bool explicit_read = !(GetFileAttributesW(io_path(root).c_str()) & FILE_ATTRIBUTE_DIRECTORY);
        std::vector<std::wstring> pending{root};
        while (!pending.empty()) {
            auto path = std::move(pending.back()); pending.pop_back(); if (++count > 200000) throw std::runtime_error("alias-tree-limit");
            DWORD attributes = GetFileAttributesW(io_path(path).c_str()); if (attributes == INVALID_FILE_ATTRIBUTES) throw std::runtime_error("alias-tree-query:" + std::to_string(GetLastError()));
            if (attributes & FILE_ATTRIBUTE_REPARSE_POINT) continue;
            if (!(attributes & FILE_ATTRIBUTE_DIRECTORY)) { aliases(path, covered(path, write), explicit_read, read, write); continue; }
            WIN32_FIND_DATAW entry{}; HANDLE search = FindFirstFileW(io_path(path + L"\\*").c_str(), &entry);
            if (search == INVALID_HANDLE_VALUE) { if (GetLastError() == ERROR_FILE_NOT_FOUND) continue; throw std::runtime_error("alias-tree-enumeration:" + std::to_string(GetLastError())); }
            do { if (wcscmp(entry.cFileName, L".") && wcscmp(entry.cFileName, L"..")) pending.push_back(path + L"\\" + entry.cFileName); } while (FindNextFileW(search, &entry));
            DWORD error = GetLastError(); FindClose(search); if (error != ERROR_NO_MORE_FILES) throw std::runtime_error("alias-tree-enumeration:" + std::to_string(error));
        }
    }
}
