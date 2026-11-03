#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include "package_authority.h"
#include <appmodel.h>
#include <bcrypt.h>
#include <array>
#include <algorithm>
#include <cstdint>
#include <utility>
#include <stdexcept>
#ifndef GENT_EXTERNAL_PACKAGE_FAMILY
#define GENT_EXTERNAL_PACKAGE_FAMILY L""
#endif
#ifndef GENT_EXTERNAL_PACKAGE_FULL_NAME
#define GENT_EXTERNAL_PACKAGE_FULL_NAME L""
#endif
#ifndef GENT_EXTERNAL_PACKAGE_IMAGE_SHA256
#define GENT_EXTERNAL_PACKAGE_IMAGE_SHA256 ""
#endif
#ifndef GENT_EXTERNAL_PACKAGE_MANIFEST_SHA256
#define GENT_EXTERNAL_PACKAGE_MANIFEST_SHA256 ""
#endif
#ifndef GENT_EXTERNAL_PACKAGE_APPLICATION
#define GENT_EXTERNAL_PACKAGE_APPLICATION L"GentNetworkProxy"
#endif
#ifndef GENT_EXTERNAL_PACKAGE_ALLOW_DEVELOPMENT_FOR_TEST
#define GENT_EXTERNAL_PACKAGE_ALLOW_DEVELOPMENT_FOR_TEST 0
#endif
#ifndef GENT_EXTERNAL_PACKAGE_IMAGE
#define GENT_EXTERNAL_PACKAGE_IMAGE L"data\\gent-network-proxy\\gent-network-proxy.exe"
#endif
namespace external_bridge {
namespace {
void require(bool ok, const char* stage) { if (!ok) throw std::runtime_error(stage); }
bool safe(const std::wstring& value) {
    return !value.empty() && value.size() <= 255 && std::all_of(value.begin(), value.end(), [](wchar_t c) {
        return (c >= L'a' && c <= L'z') || (c >= L'A' && c <= L'Z')
            || (c >= L'0' && c <= L'9') || c == L'-' || c == L'_' || c == L'.';
    });
}
std::string digest(HANDLE file, uint64_t maximum) {
    LARGE_INTEGER size{}; require(GetFileSizeEx(file, &size) && size.QuadPart > 0
        && static_cast<uint64_t>(size.QuadPart) <= maximum, "package-file-size");
    BCRYPT_ALG_HANDLE algorithm = nullptr; BCRYPT_HASH_HANDLE hash = nullptr;
    std::vector<unsigned char> object;
    require(BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) >= 0,
        "package-hash-algorithm");
    try {
        DWORD length = 0, returned = 0;
        require(BCryptGetProperty(algorithm, BCRYPT_OBJECT_LENGTH, reinterpret_cast<PUCHAR>(&length),
            sizeof(length), &returned, 0) >= 0 && length && length <= 65536, "package-hash-size");
        object.resize(length);
        require(BCryptCreateHash(algorithm, &hash, object.data(), length, nullptr, 0, 0) >= 0,
            "package-hash-create");
        std::array<unsigned char, 65536> bytes{}; uint64_t total = 0;
        for (;;) {
            DWORD read = 0;
            require(ReadFile(file, bytes.data(), static_cast<DWORD>(bytes.size()), &read, nullptr),
                "package-file-read");
            if (!read) break;
            total += read; require(total <= maximum && BCryptHashData(hash, bytes.data(), read, 0) >= 0,
                "package-hash-data");
        }
        std::array<unsigned char, 32> output{};
        require(total == static_cast<uint64_t>(size.QuadPart)
            && BCryptFinishHash(hash, output.data(), static_cast<ULONG>(output.size()), 0) >= 0,
            "package-hash-finish");
        std::string text;
        for (const auto byte : output) { text += "0123456789abcdef"[byte >> 4]; text += "0123456789abcdef"[byte & 15]; }
        BCryptDestroyHash(hash); hash = nullptr; BCryptCloseAlgorithmProvider(algorithm, 0);
        return text;
    } catch (...) {
        if (hash) BCryptDestroyHash(hash);
        BCryptCloseAlgorithmProvider(algorithm, 0); throw;
    }
}
}
PackageAuthority::PackageAuthority() {
    family = GENT_EXTERNAL_PACKAGE_FAMILY; full_name = GENT_EXTERNAL_PACKAGE_FULL_NAME;
    application = GENT_EXTERNAL_PACKAGE_APPLICATION;
    const std::wstring relative(GENT_EXTERNAL_PACKAGE_IMAGE);
    require(safe(family) && safe(full_name) && safe(application) && !relative.empty()
        && relative.size() < 1024, "package-reviewed-authority-unavailable");
    std::vector<std::wstring> components;
    for (size_t start = 0; start < relative.size();) {
        const auto end = relative.find(L'\\', start);
        auto component = relative.substr(start, end == std::wstring::npos ? end : end - start);
        require(safe(component) && component != L"." && component != L"..", "package-image-relative-path");
        components.push_back(std::move(component));
        if (end == std::wstring::npos) break;
        start = end + 1; require(start < relative.size(), "package-image-relative-trailing");
    }
    require(std::string(GENT_EXTERNAL_PACKAGE_IMAGE_SHA256).size() == 64
        && std::string(GENT_EXTERNAL_PACKAGE_MANIFEST_SHA256).size() == 64,
        "package-reviewed-digests-unavailable");
    UINT32 family_count = 0;
    require(PackageFamilyNameFromFullName(full_name.c_str(), &family_count, nullptr)
        == ERROR_INSUFFICIENT_BUFFER && family_count > 1 && family_count < 4096,
        "package-full-family-size");
    std::vector<wchar_t> derived_family(family_count);
    require(PackageFamilyNameFromFullName(full_name.c_str(), &family_count, derived_family.data())
        == ERROR_SUCCESS && std::wstring(derived_family.data()) == family, "package-full-family-mismatch");
    PACKAGE_INFO_REFERENCE reference = nullptr;
    require(OpenPackageInfoByFullName(full_name.c_str(), 0, &reference) == ERROR_SUCCESS,
        "package-registration-open");
    UINT32 info_size = 0, info_count = 0;
    const auto queried = GetPackageInfo(reference, PACKAGE_FILTER_HEAD, &info_size, nullptr, &info_count);
    if (queried != ERROR_INSUFFICIENT_BUFFER || info_size < sizeof(PACKAGE_INFO) || info_size > 65536) {
        ClosePackageInfo(reference); throw std::runtime_error("package-registration-size");
    }
    std::vector<unsigned char> info(info_size);
    const auto loaded = GetPackageInfo(reference, PACKAGE_FILTER_HEAD, &info_size, info.data(), &info_count);
    ClosePackageInfo(reference);
    require(loaded == ERROR_SUCCESS && info_count == 1, "package-registration-query");
    const auto registration = reinterpret_cast<const PACKAGE_INFO*>(info.data());
    development_registration = (registration->flags & PACKAGE_PROPERTY_DEVELOPMENT_MODE) != 0;
    require(GENT_EXTERNAL_PACKAGE_ALLOW_DEVELOPMENT_FOR_TEST
        || !development_registration,
        "package-development-registration-not-production-authority");
    UINT32 count = 0;
    require(GetPackagePathByFullName(full_name.c_str(), &count, nullptr) == ERROR_INSUFFICIENT_BUFFER
        && count > 1 && count <= 32768, "package-install-root-size");
    std::vector<wchar_t> buffer(count);
    require(GetPackagePathByFullName(full_name.c_str(), &count, buffer.data()) == ERROR_SUCCESS,
        "package-install-root");
    const std::wstring root(buffer.data());
    try {
        const auto directory = CreateFileW(root.c_str(), FILE_READ_ATTRIBUTES, FILE_SHARE_READ, nullptr,
            OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
        require(directory != INVALID_HANDLE_VALUE, "package-root-lock"); locks.push_back(directory);
        FILE_ATTRIBUTE_TAG_INFO attributes{};
        require(GetFileInformationByHandleEx(directory, FileAttributeTagInfo, &attributes, sizeof(attributes))
            && (attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY)
            && !(attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT), "package-root-physical");
        auto pin = [&](const std::wstring& path) {
            const auto file = CreateFileW(path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr,
                OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
            require(file != INVALID_HANDLE_VALUE, "package-file-lock"); locks.push_back(file);
            require(GetFileInformationByHandleEx(file, FileAttributeTagInfo, &attributes, sizeof(attributes))
                && !(attributes.FileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)),
                "package-file-physical");
            return file;
        };
        std::wstring image_parent = root;
        for (size_t index = 0; index + 1 < components.size(); ++index) {
            image_parent += L"\\" + components[index];
            const auto directory_handle = CreateFileW(image_parent.c_str(), FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ, nullptr, OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, nullptr);
            require(directory_handle != INVALID_HANDLE_VALUE, "package-image-parent-lock");
            locks.push_back(directory_handle);
            require(GetFileInformationByHandleEx(directory_handle, FileAttributeTagInfo, &attributes, sizeof(attributes))
                && (attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY)
                && !(attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT), "package-image-parent-physical");
        }
        image_path = image_parent + L"\\" + components.back(); image = pin(image_path);
        image_digest = digest(image, 16 * 1024 * 1024);
        manifest_digest = digest(pin(root + L"\\AppxManifest.xml"), 1024 * 1024);
        require(image_digest == GENT_EXTERNAL_PACKAGE_IMAGE_SHA256
            && manifest_digest == GENT_EXTERNAL_PACKAGE_MANIFEST_SHA256, "package-reviewed-digest-mismatch");
        aumid = family + L"!" + application;
    } catch (...) { for (auto handle : locks) CloseHandle(handle); locks.clear(); throw; }
}
PackageAuthority::~PackageAuthority() { for (auto handle : locks) CloseHandle(handle); }
}
