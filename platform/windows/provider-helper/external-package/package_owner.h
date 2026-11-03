#pragma once
#include <windows.h>
#include <string>
#include <vector>
namespace external_bridge {
class PackageOwner {
    HANDLE parent = INVALID_HANDLE_VALUE, receipt = INVALID_HANDLE_VALUE;
    std::vector<HANDLE> namespace_locks;
public:
    HANDLE job = nullptr;
    PSID sid = nullptr;
    DWORD session = 0;
    std::wstring sid_text, generation, descriptor, observation, control_root;
    PackageOwner(const std::wstring& family, const std::wstring& state_root,
                 const std::wstring& nonce);
    ~PackageOwner();
    PackageOwner(const PackageOwner&) = delete;
    PackageOwner& operator=(const PackageOwner&) = delete;
    void record(const std::string& metadata);
    bool job_empty() const;
};
}
