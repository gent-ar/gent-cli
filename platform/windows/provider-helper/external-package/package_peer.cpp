#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <appmodel.h>
#include <sddl.h>
#include <vector>
#include <stdexcept>
#include <cstring>
#include "package_peer.h"

namespace external_bridge {
namespace {
struct Handle {
    HANDLE value;
    ~Handle() { if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value); }
};
void require(bool value, const char* stage) {
    if (!value) throw std::runtime_error(stage);
}
using PackageQuery = LONG (WINAPI*)(HANDLE, UINT32*, PWSTR);
std::wstring package_string(HANDLE process, PackageQuery query) {
    UINT32 count = 0;
    require(query(process, &count, nullptr) == ERROR_INSUFFICIENT_BUFFER
        && count > 1 && count <= 4096, "external-package-size");
    std::vector<wchar_t> buffer(count);
    require(query(process, &count, buffer.data()) == ERROR_SUCCESS
        && count > 1 && count <= buffer.size() && buffer[count - 1] == 0,
        "external-package-query");
    return std::wstring(buffer.data(), count - 1);
}
bool same_file(HANDLE first, HANDLE second) {
    FILE_ID_INFO a{}, b{};
    return GetFileInformationByHandleEx(first, FileIdInfo, &a, sizeof(a))
        && GetFileInformationByHandleEx(second, FileIdInfo, &b, sizeof(b))
        && a.VolumeSerialNumber == b.VolumeSerialNumber
        && memcmp(&a.FileId, &b.FileId, sizeof(a.FileId)) == 0;
}
}
HANDLE capture_package_peer(const PackageGrant& grant) {
    require(grant.pid && grant.creation && !grant.family.empty()
        && !grant.full_name.empty() && !grant.appcontainer_sid.empty()
        && grant.pinned_image && grant.pinned_image != INVALID_HANDLE_VALUE,
        "external-grant-incomplete");
    Handle process{OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION
        | PROCESS_DUP_HANDLE | SYNCHRONIZE, FALSE, grant.pid)};
    require(process.value && WaitForSingleObject(process.value, 0) == WAIT_TIMEOUT,
        "external-peer-open-or-exited");
    FILETIME created{}, exited{}, kernel{}, user{};
    require(GetProcessTimes(process.value, &created, &exited, &kernel, &user) != 0,
        "external-peer-birth-query");
    ULARGE_INTEGER birth{};
    birth.LowPart = created.dwLowDateTime; birth.HighPart = created.dwHighDateTime;
    DWORD session = 0, own_session = 0;
    require(birth.QuadPart == grant.creation
        && ProcessIdToSessionId(grant.pid, &session)
        && ProcessIdToSessionId(GetCurrentProcessId(), &own_session)
        && session == grant.session && session == own_session,
        "external-peer-birth-session");
    require(package_string(process.value, GetPackageFamilyName) == grant.family
        && package_string(process.value, GetPackageFullName) == grant.full_name,
        "external-peer-package-mismatch");
    Handle token{nullptr};
    require(OpenProcessToken(process.value, TOKEN_QUERY, &token.value) != 0,
        "external-peer-token");
    Handle own_token{nullptr};
    require(OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &own_token.value) != 0,
        "external-owner-token");
    DWORD peer_user_size = 0, own_user_size = 0;
    GetTokenInformation(token.value, TokenUser, nullptr, 0, &peer_user_size);
    GetTokenInformation(own_token.value, TokenUser, nullptr, 0, &own_user_size);
    require(peer_user_size && peer_user_size <= 65536
        && own_user_size && own_user_size <= 65536, "external-user-size");
    std::vector<unsigned char> peer_user(peer_user_size), own_user(own_user_size);
    require(GetTokenInformation(token.value, TokenUser, peer_user.data(),
            peer_user_size, &peer_user_size)
        && GetTokenInformation(own_token.value, TokenUser, own_user.data(),
            own_user_size, &own_user_size), "external-user-query");
    const auto peer_user_sid = reinterpret_cast<TOKEN_USER*>(peer_user.data())->User.Sid;
    const auto own_user_sid = reinterpret_cast<TOKEN_USER*>(own_user.data())->User.Sid;
    require(IsValidSid(peer_user_sid) && IsValidSid(own_user_sid)
        && EqualSid(peer_user_sid, own_user_sid), "external-peer-user-mismatch");
    DWORD app = 0, size = 0;
    require(GetTokenInformation(token.value, TokenIsAppContainer,
        &app, sizeof(app), &size) && app == 1, "external-peer-not-appcontainer");
    GetTokenInformation(token.value, TokenAppContainerSid, nullptr, 0, &size);
    require(size && size <= 65536, "external-peer-sid-size");
    std::vector<unsigned char> information(size);
    require(GetTokenInformation(token.value, TokenAppContainerSid,
        information.data(), size, &size) != 0, "external-peer-sid-query");
    auto sid = reinterpret_cast<TOKEN_APPCONTAINER_INFORMATION*>(information.data());
    LPWSTR text = nullptr;
    require(sid->TokenAppContainer && IsValidSid(sid->TokenAppContainer)
        && ConvertSidToStringSidW(sid->TokenAppContainer, &text), "external-peer-sid");
    const std::wstring actual_sid(text); LocalFree(text);
    require(actual_sid == grant.appcontainer_sid, "external-peer-sid-mismatch");
    std::vector<wchar_t> path(32768); DWORD length = static_cast<DWORD>(path.size());
    require(QueryFullProcessImageNameW(process.value, 0, path.data(), &length)
        && length && length < path.size(), "external-peer-image-path");
    Handle image{CreateFileW(path.data(), FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
        nullptr, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr)};
    FILE_ATTRIBUTE_TAG_INFO attributes{};
    require(image.value != INVALID_HANDLE_VALUE
        && GetFileInformationByHandleEx(image.value, FileAttributeTagInfo,
            &attributes, sizeof(attributes))
        && !(attributes.FileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT))
        && same_file(image.value, grant.pinned_image), "external-peer-image-mismatch");
    require(WaitForSingleObject(process.value, 0) == WAIT_TIMEOUT,
        "external-peer-exited-after-validation");
    const auto result = process.value; process.value = nullptr; return result;
}
}
