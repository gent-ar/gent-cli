#include <winsock2.h>
#include <windows.h>
#include <bcrypt.h>
#include <wintrust.h>
#include <softpub.h>
#include <mscat.h>
#include <sfc.h>
#include <iphlpapi.h>
#include <algorithm>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>
#include "resource_inspection.h"

namespace {
void require(bool ok, const char* label) { if (!ok) throw std::runtime_error(label); }
struct File {
    HANDLE value = INVALID_HANDLE_VALUE;
    explicit File(const std::wstring& path) : value(CreateFileW(path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, nullptr)) { require(value != INVALID_HANDLE_VALUE, "inspect-file-open"); }
    ~File() { CloseHandle(value); }
    File(const File&) = delete;
    File& operator=(const File&) = delete;
};
std::string json(const std::wstring& value) {
    const int count = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(), static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
    require(count > 0, "inspect-unicode"); std::string encoded(static_cast<size_t>(count), '\0');
    require(WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(), static_cast<int>(value.size()), encoded.data(), count, nullptr, nullptr) == count, "inspect-unicode");
    std::string result = "\"";
    for (unsigned char c : encoded) { require(c >= 32, "inspect-json-control"); if (c == '"' || c == '\\') result += '\\'; result += static_cast<char>(c); }
    return result + "\"";
}
std::wstring canonical(HANDLE file) {
    std::vector<wchar_t> path(32768); const DWORD count = GetFinalPathNameByHandleW(file, path.data(), static_cast<DWORD>(path.size()), FILE_NAME_NORMALIZED | VOLUME_NAME_DOS);
    require(count && count < path.size(), "inspect-canonical"); return std::wstring(path.data(), count);
}
std::string hash(HANDLE file) {
    BCRYPT_ALG_HANDLE algorithm = nullptr; BCRYPT_HASH_HANDLE digest = nullptr;
    require(BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) >= 0, "inspect-hash-provider");
    struct Cleanup { BCRYPT_ALG_HANDLE a; BCRYPT_HASH_HANDLE& h; ~Cleanup() { if (h) BCryptDestroyHash(h); BCryptCloseAlgorithmProvider(a, 0); } } cleanup{algorithm, digest};
    require(BCryptCreateHash(algorithm, &digest, nullptr, 0, nullptr, 0, 0) >= 0, "inspect-hash-create");
    LARGE_INTEGER zero{}; require(SetFilePointerEx(file, zero, nullptr, FILE_BEGIN) != 0, "inspect-rewind");
    std::vector<UCHAR> bytes(65536); DWORD count = 0;
    do { require(ReadFile(file, bytes.data(), static_cast<DWORD>(bytes.size()), &count, nullptr) != 0, "inspect-read"); if (count) require(BCryptHashData(digest, bytes.data(), count, 0) >= 0, "inspect-hash-data"); } while (count);
    UCHAR result[32]{}; require(BCryptFinishHash(digest, result, sizeof(result), 0) >= 0, "inspect-hash-finish");
    const char* hex = "0123456789abcdef"; std::string text; for (UCHAR c : result) { text += hex[c >> 4]; text += hex[c & 15]; } return text;
}
void nonce(const std::wstring& text) {
    require(text.size() == 36 && std::all_of(text.begin(), text.end(), [](wchar_t c) { return (c >= L'a' && c <= L'f') || (c >= L'0' && c <= L'9') || c == L'-'; }), "inspect-nonce");
}
bool trusted(WINTRUST_DATA& data) {
    GUID action = WINTRUST_ACTION_GENERIC_VERIFY_V2; data.dwStateAction = WTD_STATEACTION_VERIFY;
    const LONG result = WinVerifyTrust(nullptr, &action, &data);
    bool microsoft = false;
    if (result == ERROR_SUCCESS) {
        auto provider = WTHelperProvDataFromStateData(data.hWVTStateData);
        auto signer = provider ? WTHelperGetProvSignerFromChain(provider, 0, FALSE, 0) : nullptr;
        auto cert = signer ? WTHelperGetProvCertFromChain(signer, 0) : nullptr;
        if (cert && cert->pCert) {
            wchar_t organization[256]{};
            const DWORD length = CertGetNameStringW(cert->pCert, CERT_NAME_ATTR_TYPE, 0, const_cast<char*>(szOID_ORGANIZATION_NAME), organization, 256);
            microsoft = length > 1 && length <= 256 && std::wstring(organization) == L"Microsoft Corporation";
        }
    }
    data.dwStateAction = WTD_STATEACTION_CLOSE; WinVerifyTrust(nullptr, &action, &data); return microsoft;
}
bool signed_by_microsoft(const std::wstring& path, HANDLE file) {
    WINTRUST_FILE_INFO info{}; info.cbStruct = sizeof(info); info.pcwszFilePath = path.c_str(); info.hFile = file;
    WINTRUST_DATA data{}; data.cbStruct = sizeof(data); data.dwUIChoice = WTD_UI_NONE; data.fdwRevocationChecks = WTD_REVOKE_WHOLECHAIN; data.dwUnionChoice = WTD_CHOICE_FILE; data.pFile = &info;
    data.dwProvFlags = WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT;
    if (trusted(data)) return true;
    // These modern catalog APIs are resolved from the already-loaded system
    // Wintrust image, as required by the Windows API contract.
    const auto library = GetModuleHandleW(L"wintrust.dll");
    if (!library) return false;
    const auto acquire = reinterpret_cast<decltype(&CryptCATAdminAcquireContext2)>(GetProcAddress(library, "CryptCATAdminAcquireContext2"));
    const auto calculate = reinterpret_cast<decltype(&CryptCATAdminCalcHashFromFileHandle2)>(GetProcAddress(library, "CryptCATAdminCalcHashFromFileHandle2"));
    if (!acquire || !calculate) return false;
    HCATADMIN admin = nullptr; GUID subsystem = DRIVER_ACTION_VERIFY;
    if (!acquire(&admin, &subsystem, BCRYPT_SHA256_ALGORITHM, nullptr, 0)) return false;
    struct Admin { HCATADMIN value; ~Admin() { CryptCATAdminReleaseContext(value, 0); } } owner{admin};
    LARGE_INTEGER start{}; if (!SetFilePointerEx(file, start, nullptr, FILE_BEGIN)) return false;
    DWORD size = 0;
    if (!calculate(admin, file, &size, nullptr, 0) || !size || size > 128) return false;
    std::vector<BYTE> digest(size); if (!calculate(admin, file, &size, digest.data(), 0)) return false;
    const wchar_t* hex = L"0123456789ABCDEF"; std::wstring tag; for (BYTE c : digest) { tag += hex[c >> 4]; tag += hex[c & 15]; }
    HCATINFO catalog = CryptCATAdminEnumCatalogFromHash(admin, digest.data(), size, 0, nullptr);
    if (!catalog) return false;
    struct Catalog { HCATADMIN admin; HCATINFO value; ~Catalog() { CryptCATAdminReleaseCatalogContext(admin, value, 0); } } catalog_owner{admin, catalog};
    CATALOG_INFO catalog_info{}; catalog_info.cbStruct = sizeof(catalog_info); if (!CryptCATCatalogInfoFromContext(catalog, &catalog_info, 0)) return false;
    WINTRUST_CATALOG_INFO membership{}; membership.cbStruct = sizeof(membership); membership.pcwszCatalogFilePath = catalog_info.wszCatalogFile; membership.pcwszMemberTag = tag.c_str(); membership.pcwszMemberFilePath = path.c_str(); membership.hMemberFile = file; membership.pbCalculatedFileHash = digest.data(); membership.cbCalculatedFileHash = size; membership.hCatAdmin = admin;
    data = {}; data.cbStruct = sizeof(data); data.dwUIChoice = WTD_UI_NONE; data.fdwRevocationChecks = WTD_REVOKE_WHOLECHAIN; data.dwUnionChoice = WTD_CHOICE_CATALOG; data.pCatalog = &membership; data.dwProvFlags = WTD_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT;
    return trusted(data);
}
bool protected_file(const std::wstring& path) {
    const auto native = path.rfind(L"\\\\?\\", 0) == 0 ? path.substr(4) : path;
    return SfcIsFileProtected(nullptr, native.c_str()) != 0;
}
std::wstring system_directory() {
    wchar_t path[MAX_PATH]{}; const UINT count = GetSystemDirectoryW(path, MAX_PATH); require(count && count < MAX_PATH, "inspect-system-directory");
    HANDLE directory = CreateFileW(path, FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, nullptr);
    require(directory != INVALID_HANDLE_VALUE, "inspect-system-directory-open");
    const auto result = canonical(directory); CloseHandle(directory); return result;
}
bool dll_name(const std::wstring& name) {
    return name.size() > 4 && name.size() < 128 && name.substr(name.size() - 4) == L".dll" && name.find(L"..") == std::wstring::npos && std::all_of(name.begin(), name.end(), [](wchar_t c) { return (c >= L'a' && c <= L'z') || (c >= L'0' && c <= L'9') || c == L'.' || c == L'_' || c == L'-'; });
}
}
int inspect_os_resources(int argc, wchar_t** argv) {
    require(argc >= 3 && argc <= 131, "inspect-os-arguments"); nonce(argv[2]);
    const auto root = system_directory();
    // The signed, protected kernel identity changes with Windows servicing.
    File kernel(root + L"\\kernel32.dll"); require(protected_file(root + L"\\kernel32.dll") && signed_by_microsoft(root + L"\\kernel32.dll", kernel.value), "inspect-os-provenance");
    std::string report = "{\"schema\":1,\"nonce\":" + json(argv[2]) + ",\"systemDirectory\":" + json(root) + ",\"osIdentity\":\"kernel32-sha256:" + hash(kernel.value) + "\",\"libraries\":[";
    std::vector<std::wstring> seen;
    for (int i = 3; i < argc; ++i) {
        const std::wstring name = argv[i]; require(dll_name(name) && std::find(seen.begin(), seen.end(), name) == seen.end(), "inspect-os-name"); seen.push_back(name);
        const auto path = root + L"\\" + name; File file(path); const auto measured = canonical(file.value);
        require(_wcsicmp(measured.c_str(), path.c_str()) == 0 && protected_file(path) && signed_by_microsoft(path, file.value), "inspect-os-library-provenance");
        if (i != 3) report += ',';
        report += "{\"name\":" + json(name) + ",\"path\":" + json(measured) + ",\"sha256\":\"" + hash(file.value) + "\"}";
    }
    std::cout << report << "]}\n"; return 0;
}
int inspect_model_endpoint(int argc, wchar_t** argv) {
    require(argc == 5, "inspect-model-arguments"); nonce(argv[2]);
    const auto pid = std::stoull(argv[3]), port = std::stoull(argv[4]); require(pid && pid <= MAXDWORD && port <= 65535, "inspect-model-identity");
    if (port != 0) {
    DWORD size = 0; require(GetExtendedTcpTable(nullptr, &size, FALSE, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0) == ERROR_INSUFFICIENT_BUFFER && size <= 1024 * 1024, "inspect-tcp-size");
    std::vector<BYTE> table(size); require(GetExtendedTcpTable(table.data(), &size, FALSE, AF_INET, TCP_TABLE_OWNER_PID_LISTENER, 0) == NO_ERROR, "inspect-tcp-table");
    const auto rows = reinterpret_cast<MIB_TCPTABLE_OWNER_PID*>(table.data()); bool found = false;
    for (DWORD i = 0; i < rows->dwNumEntries; ++i) { const auto& row = rows->table[i]; if (row.dwOwningPid == pid && row.dwLocalAddr == htonl(INADDR_LOOPBACK) && ntohs(static_cast<u_short>(row.dwLocalPort)) == port) found = true; }
    require(found, "inspect-model-listener-owner");
    }
    HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, static_cast<DWORD>(pid)); require(process != nullptr, "inspect-model-process");
    FILETIME created{}, exited{}, kernel{}, user{}; const bool measured = GetProcessTimes(process, &created, &exited, &kernel, &user) != 0; CloseHandle(process); require(measured, "inspect-model-created");
    ULARGE_INTEGER time{}; time.LowPart = created.dwLowDateTime; time.HighPart = created.dwHighDateTime;
    std::cout << "{\"schema\":1,\"nonce\":" << json(argv[2]) << ",\"pid\":" << pid << ",\"port\":" << port << ",\"created\":" << time.QuadPart << "}\n"; return 0;
}

int inspect_globalization_data(int argc, wchar_t** argv) {
    require(argc == 3, "inspect-globalization-arguments"); nonce(argv[2]);
    const auto system = system_directory();
    File kernel(system + L"\\kernel32.dll");
    require(protected_file(system + L"\\kernel32.dll") &&
        signed_by_microsoft(system + L"\\kernel32.dll", kernel.value), "inspect-globalization-os");
    const auto path = system.substr(0, system.find_last_of(L'\\')) + L"\\Globalization\\Sorting\\sortdefault.nls";
    File data(path); const auto measured = canonical(data.value);
    require(_wcsicmp(measured.c_str(), path.c_str()) == 0 && protected_file(path), "inspect-globalization-provenance");
    const std::string report = R"({"schema":1,"nonce":)" + json(argv[2]) +
        R"(,"path":)" + json(measured) + R"(,"sha256":")" + hash(data.value) + R"(","protected":true})" + "\n";
    require(report.size() <= 4096, "inspect-globalization-bound");
    std::cout << report; return 0;
}
