#pragma once
#include <windows.h>
#include <string>
#include <vector>
namespace external_bridge {
class PackageAuthority {
    std::vector<HANDLE> locks;
public:
    std::wstring family, full_name, image_path, aumid, application;
    bool development_registration = false;
    std::string image_digest, manifest_digest;
    HANDLE image = INVALID_HANDLE_VALUE;
    PackageAuthority();
    ~PackageAuthority();
    PackageAuthority(const PackageAuthority&) = delete;
    PackageAuthority& operator=(const PackageAuthority&) = delete;
};
}
