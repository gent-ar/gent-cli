#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <string>
#include <vector>
#include <stdexcept>
#include <iostream>
#include <aclapi.h>
#include <sddl.h>
#include <array>
#include "win32_paths.h"
#include "filesystem_aliases.h"
#include "readonly_authority.h"
namespace after {
#include "filesystem_aliases.cpp"
}
namespace authority {
#include "readonly_authority.cpp"
}
static void require(bool value, const char* label) {
    if (!value) throw std::runtime_error(label);
}
static void directory(const std::wstring& path) {
    require(CreateDirectoryW(path.c_str(), nullptr) != 0, "owned-directory");
}
static void file(const std::wstring& path) {
    HANDLE handle = CreateFileW(path.c_str(), GENERIC_WRITE, 7, nullptr, CREATE_NEW, 0, nullptr);
    require(handle != INVALID_HANDLE_VALUE, "owned-file"); CloseHandle(handle);
}
static void readonly_checks(const std::wstring& writable_file, const std::wstring& root) {
    HANDLE token = nullptr;
    require(OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY | TOKEN_DUPLICATE, &token) != 0, "test-token");
    try {
        bool writable_rejected = false;
        try { authority::validate_readonly_authority(token, {writable_file}, {}); }
        catch (const std::exception& error) { writable_rejected = std::string(error.what()) == "readonly-ambient-write-authority"; }
        require(writable_rejected, "long-writable-authority-rejected");
        authority::validate_readonly_authority(token, {writable_file}, {root.substr(4)});
        DWORD size = 0;
        GetTokenInformation(token, TokenUser, nullptr, 0, &size);
        std::vector<unsigned char> user(size);
        require(size && GetTokenInformation(token, TokenUser, user.data(), size, &size), "test-user");
        LPWSTR sid = nullptr;
        require(ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(user.data())->User.Sid, &sid), "test-sid");
        const std::wstring sddl = L"D:P(A;;FR;;;" + std::wstring(sid) + L")(A;;RC;;;OW)";
        LocalFree(sid);
        PSECURITY_DESCRIPTOR descriptor = nullptr;
        require(ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(), SDDL_REVISION_1, &descriptor, nullptr), "test-readonly-security");
        SECURITY_ATTRIBUTES attributes{sizeof(attributes), descriptor, FALSE};
        const auto readonly_file = root + L"\\readonly.txt";
        HANDLE readonly_handle = CreateFileW(readonly_file.c_str(), GENERIC_READ, 7, &attributes, CREATE_NEW, 0, nullptr);
        const auto error = GetLastError(); LocalFree(descriptor); SetLastError(error);
        require(readonly_handle != INVALID_HANDLE_VALUE, "test-readonly-file"); CloseHandle(readonly_handle);
        authority::validate_readonly_authority(token, {readonly_file}, {});
        authority::validate_readonly_authority(token, {readonly_file.substr(4)}, {});
        std::cout << "PASS readonly-long-positive ambient-write-negative explicit-write-exclusion\n";
    } catch (...) { CloseHandle(token); throw; }
    CloseHandle(token);
}
int wmain(int argc, wchar_t** argv) {
    try {
        require(argc == 2, "owned-scope-argument");
        std::wstring root = L"\\\\?\\" + std::wstring(argv[1]) + L"-" + std::to_wstring(GetCurrentProcessId()) + L"-" + std::to_wstring(GetTickCount64());
        directory(root);
        auto long_root = root;
        while (long_root.size() < 290) {
            long_root += L"\\segment-0123456789abcdef"; directory(long_root);
        }
        const auto long_file = long_root + L"\\grant.txt"; file(long_file);
        after::validate_filesystem_aliases({long_file}, {});
        readonly_checks(long_file, long_root);
        const auto short_file = root + L"\\short.txt"; file(short_file);
        after::validate_filesystem_aliases({short_file}, {});
        require(gent_windows_paths::comparison_path(L"\\\\?\\UNC\\server\\share\\item") == L"\\\\server\\share\\item", "unc-comparison");
        require(gent_windows_paths::io_path(L"\\\\server\\share\\item") == L"\\\\?\\UNC\\server\\share\\item", "unc-io-only-no-network");
        require(gent_windows_paths::comparison_path(long_file) == gent_windows_paths::comparison_path(long_file.substr(4)), "same-prefix-comparison");
        bool relative_rejected = false;
        try { gent_windows_paths::io_path(L"relative\\file"); }
        catch (...) { relative_rejected = true; }
        require(relative_rejected, "relative-path-rejected");
        const auto writable = root + L"\\write"; directory(writable);
        const auto outside = root + L"\\outside.txt"; file(outside);
        const auto inside = writable + L"\\alias.txt";
        require(CreateHardLinkW(inside.c_str(), outside.c_str(), nullptr) != 0, "owned-hardlink");
        bool escape_rejected = false;
        try { after::validate_filesystem_aliases({writable}, {writable}); }
        catch (const std::exception& error) { escape_rejected = std::string(error.what()) == "filesystem-hardlink-escape"; }
        require(escape_rejected, "hardlink-outside-grant-rejected");
        after::validate_filesystem_aliases({root}, {root});
        std::cout << "PASS long-readonly short-readonly prefix-equivalence unc-pure relative-negative hardlink-negative owned-all-aliases\n";
        return 0;
    } catch (const std::exception& error) {
        std::cerr << error.what() << "\n"; return 1;
    }
}
