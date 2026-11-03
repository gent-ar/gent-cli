#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include "package_owner.h"
#include "external_channel_client.h"
#include <shlobj.h>
#include <aclapi.h>
#include <sddl.h>
#include <userenv.h>
#include <stdexcept>
namespace external_bridge {
namespace {
void require(bool ok, const char* stage) { if (!ok) throw std::runtime_error(stage); }
void owner_access(PSID sid) {
    PACL prior = nullptr, next = nullptr; PSECURITY_DESCRIPTOR security = nullptr;
    DWORD error = GetSecurityInfo(GetCurrentProcess(), SE_KERNEL_OBJECT, DACL_SECURITY_INFORMATION,
        nullptr, nullptr, &prior, nullptr, &security);
    EXPLICIT_ACCESSW access{};
    access.grfAccessPermissions = SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
    access.grfAccessMode = GRANT_ACCESS; access.Trustee.TrusteeForm = TRUSTEE_IS_SID;
    access.Trustee.ptstrName = static_cast<LPWSTR>(sid);
    if (!error) error = SetEntriesInAclW(1, &access, prior, &next);
    if (!error) error = SetSecurityInfo(GetCurrentProcess(), SE_KERNEL_OBJECT,
        DACL_SECURITY_INFORMATION, nullptr, nullptr, next, nullptr);
    if (next) LocalFree(next); if (security) LocalFree(security);
    require(!error, "package-owner-query-grant");
}
HANDLE physical_directory(const std::wstring& path) {
    const auto handle = CreateFileW(path.c_str(), FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE,
        nullptr, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
    require(handle != INVALID_HANDLE_VALUE, "package-state-directory-open");
    FILE_ATTRIBUTE_TAG_INFO info{};
    const bool ok = GetFileInformationByHandleEx(handle, FileAttributeTagInfo, &info, sizeof(info))
        && (info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY)
        && !(info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT);
    if (!ok) CloseHandle(handle);
    require(ok, "package-state-directory-physical"); return handle;
}
std::wstring current_owner_sid() {
    HANDLE token = nullptr;
    require(OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token), "package-control-owner-token");
    DWORD size = 0; GetTokenInformation(token, TokenUser, nullptr, 0, &size);
    if (!size || size > 65536) { CloseHandle(token); throw std::runtime_error("package-control-owner-size"); }
    std::vector<unsigned char> value(size);
    const bool read = GetTokenInformation(token, TokenUser, value.data(), size, &size) != 0;
    CloseHandle(token); require(read, "package-control-owner-query");
    LPWSTR text = nullptr;
    require(ConvertSidToStringSidW(reinterpret_cast<TOKEN_USER*>(value.data())->User.Sid, &text),
        "package-control-owner-sid");
    const std::wstring result(text); LocalFree(text); return result;
}
}
PackageOwner::PackageOwner(const std::wstring& family, const std::wstring& state_root,
    const std::wstring& nonce) {
    try {
        require(nonce.size() == 32 && std::all_of(nonce.begin(), nonce.end(), [](wchar_t c) {
            return (c >= L'0' && c <= L'9') || (c >= L'a' && c <= L'f');
        }), "package-owner-nonce");
        require(SUCCEEDED(DeriveAppContainerSidFromAppContainerName(family.c_str(), &sid)),
            "package-owner-sid");
        LPWSTR text = nullptr;
        require(ConvertSidToStringSidW(sid, &text), "package-owner-sid-text");
        sid_text = text; LocalFree(text); owner_access(sid);
        require(ProcessIdToSessionId(GetCurrentProcessId(), &session), "package-owner-session");
        parent = CreateFileW(state_root.c_str(), FILE_READ_ATTRIBUTES, FILE_SHARE_READ, nullptr,
            OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
        require(parent != INVALID_HANDLE_VALUE, "package-owner-root-lock");
        FILE_ATTRIBUTE_TAG_INFO info{};
        require(GetFileInformationByHandleEx(parent, FileAttributeTagInfo, &info, sizeof(info))
            && (info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY)
            && !(info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT), "package-owner-root-physical");
        PWSTR local = nullptr;
        require(SUCCEEDED(SHGetKnownFolderPath(FOLDERID_LocalAppData, KF_FLAG_DEFAULT, nullptr, &local)),
            "package-local-state-root");
        std::wstring base(local); CoTaskMemFree(local);
        namespace_locks.push_back(physical_directory(base));
        // Control authority is outside state_root (which the provider can write)
        // and outside package LocalState. No package/ALL_APP ACE is granted.
        control_root = base + L"\\GentProxyOwner-" + nonce;
        const auto private_sddl = L"D:P(A;OICI;FA;;;" + current_owner_sid() + L")";
        PSECURITY_DESCRIPTOR private_security = nullptr;
        require(ConvertStringSecurityDescriptorToSecurityDescriptorW(private_sddl.c_str(), SDDL_REVISION_1,
            &private_security, nullptr), "package-control-security");
        SECURITY_ATTRIBUTES private_attributes{sizeof(private_attributes), private_security, FALSE};
        const bool fresh_control = CreateDirectoryW(control_root.c_str(), &private_attributes) != 0;
        LocalFree(private_security); require(fresh_control, "package-control-must-be-fresh");
        namespace_locks.push_back(physical_directory(control_root));
        receipt = CreateFileW((control_root + L"\\owner.receipt").c_str(), GENERIC_WRITE,
            FILE_SHARE_READ, nullptr, CREATE_NEW, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_WRITE_THROUGH, nullptr);
        require(receipt != INVALID_HANDLE_VALUE, "package-owner-receipt-existing-or-unavailable");
        record("GENT-EXTERNAL-OWNER/1\nPendingActivation " + std::to_string(GetCurrentProcessId())
            + " " + std::to_string(process_birth(GetCurrentProcess())) + "\n");
        for (const auto& component : {L"Packages", family.c_str(), L"LocalState"}) {
            base += L"\\"; base += component;
            if (!CreateDirectoryW(base.c_str(), nullptr)) require(GetLastError() == ERROR_ALREADY_EXISTS,
                "package-local-state-create");
            namespace_locks.push_back(physical_directory(base));
        }
        generation = base + L"\\GentProxy-" + nonce;
        const auto sddl = L"D:P(A;OICI;FA;;;" + current_owner_sid()
            + L")(A;OICI;FA;;;" + sid_text + L")";
        PSECURITY_DESCRIPTOR descriptor_security = nullptr;
        require(ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(), SDDL_REVISION_1,
            &descriptor_security, nullptr), "package-generation-security");
        SECURITY_ATTRIBUTES attributes{sizeof(attributes), descriptor_security, FALSE};
        const bool created = CreateDirectoryW(generation.c_str(), &attributes) != 0;
        LocalFree(descriptor_security); require(created, "package-generation-must-be-fresh");
        namespace_locks.push_back(physical_directory(generation));
        descriptor = generation + L"\\channels"; observation = generation + L"\\observation.json";
        job = CreateJobObjectW(nullptr, nullptr); require(job != nullptr, "package-job-create");
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits{};
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        limits.ProcessMemoryLimit = 128 * 1024 * 1024;
        require(SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits, sizeof(limits)),
            "package-job-limits");
    } catch (...) {
        for (auto handle : namespace_locks) CloseHandle(handle); namespace_locks.clear();
        if (job) CloseHandle(job); job = nullptr;
        if (receipt != INVALID_HANDLE_VALUE) CloseHandle(receipt); receipt = INVALID_HANDLE_VALUE;
        if (parent != INVALID_HANDLE_VALUE) CloseHandle(parent); parent = INVALID_HANDLE_VALUE;
        if (sid) FreeSid(sid); sid = nullptr; throw;
    }
}
PackageOwner::~PackageOwner() {
    if (job) CloseHandle(job);
    if (receipt != INVALID_HANDLE_VALUE) CloseHandle(receipt);
    if (parent != INVALID_HANDLE_VALUE) CloseHandle(parent);
    if (sid) FreeSid(sid);
    for (auto handle : namespace_locks) CloseHandle(handle);
}
void PackageOwner::record(const std::string& metadata) {
    require(!metadata.empty() && metadata.size() <= 4096, "package-receipt-bound");
    DWORD written = 0;
    require(WriteFile(receipt, metadata.data(), static_cast<DWORD>(metadata.size()), &written, nullptr)
        && written == metadata.size() && FlushFileBuffers(receipt), "package-receipt-flush");
}
bool PackageOwner::job_empty() const {
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION value{};
    return job && QueryInformationJobObject(job, JobObjectBasicAccountingInformation,
        &value, sizeof(value), nullptr) && value.ActiveProcesses == 0;
}
}
