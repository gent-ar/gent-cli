#include "readonly_authority.h"
#include "win32_paths.h"
#include <aclapi.h>
#include <array>
#include <stdexcept>

using gent_windows_paths::comparison_path;
using gent_windows_paths::io_path;
static bool within(const std::wstring& path, const std::wstring& root) {
    return path.size() >= root.size() && _wcsnicmp(path.c_str(), root.c_str(), root.size()) == 0
        && (path.size() == root.size() || path[root.size()] == L'\\');
}
static bool writable_by_acl(HANDLE token, const std::wstring& path) {
    PSECURITY_DESCRIPTOR security = nullptr;
    const auto native = io_path(path);
    DWORD result = GetNamedSecurityInfoW(const_cast<LPWSTR>(native.c_str()), SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
        nullptr, nullptr, nullptr, nullptr, &security);
    if (result != ERROR_SUCCESS) throw std::runtime_error("readonly-acl-query:" + std::to_string(result));
    GENERIC_MAPPING mapping{FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_GENERIC_EXECUTE, FILE_ALL_ACCESS};
    std::array<uint8_t, 1024> privileges{}; DWORD length = static_cast<DWORD>(privileges.size()), granted = 0; BOOL allowed = FALSE;
    BOOL queried = AccessCheck(security, token, MAXIMUM_ALLOWED, &mapping, reinterpret_cast<PRIVILEGE_SET*>(privileges.data()), &length, &granted, &allowed);
    LocalFree(security);
    if (!queried) throw std::runtime_error("readonly-access-check:" + std::to_string(GetLastError()));
    constexpr DWORD changes = FILE_WRITE_DATA | FILE_APPEND_DATA | FILE_WRITE_EA | FILE_WRITE_ATTRIBUTES | FILE_DELETE_CHILD | DELETE | WRITE_DAC | WRITE_OWNER;
    return allowed && (granted & changes);
}
void validate_readonly_authority(HANDLE primary, const std::vector<std::wstring>& readable, const std::vector<std::wstring>& writable) {
    HANDLE token = nullptr;
    if (!DuplicateToken(primary, SecurityIdentification, &token)) throw std::runtime_error("readonly-token:" + std::to_string(GetLastError()));
    try {
        std::vector<std::wstring> excluded; for (const auto& root : writable) excluded.push_back(comparison_path(root));
        size_t checked = 0;
        for (const auto& root : readable) {
            std::vector<std::wstring> pending{comparison_path(root)};
            while (!pending.empty()) {
                auto path = std::move(pending.back()); pending.pop_back(); bool excluded_path = false;
                for (const auto& writable_root : excluded) if (within(path, writable_root)) { excluded_path = true; break; }
                if (excluded_path) continue;
                if (++checked > 200000) throw std::runtime_error("readonly-tree-limit");
                DWORD attributes = GetFileAttributesW(io_path(path).c_str());
                if (attributes == INVALID_FILE_ATTRIBUTES) throw std::runtime_error("readonly-tree-query:" + std::to_string(GetLastError()));
                if (attributes & FILE_ATTRIBUTE_REPARSE_POINT) throw std::runtime_error("readonly-tree-reparse");
                if (writable_by_acl(token, path)) throw std::runtime_error("readonly-ambient-write-authority");
                if (!(attributes & FILE_ATTRIBUTE_DIRECTORY)) continue;
                WIN32_FIND_DATAW data{}; HANDLE search = FindFirstFileW(io_path(path + L"\\*").c_str(), &data);
                if (search == INVALID_HANDLE_VALUE) { if (GetLastError() == ERROR_FILE_NOT_FOUND) continue; throw std::runtime_error("readonly-enumeration:" + std::to_string(GetLastError())); }
                do { if (wcscmp(data.cFileName, L".") && wcscmp(data.cFileName, L"..")) pending.push_back(path + L"\\" + data.cFileName); } while (FindNextFileW(search, &data));
                DWORD error = GetLastError(); FindClose(search);
                if (error != ERROR_NO_MORE_FILES) throw std::runtime_error("readonly-enumeration:" + std::to_string(error));
            }
        }
    } catch (...) { CloseHandle(token); throw; }
    CloseHandle(token);
}
