#pragma once
#include <windows.h>
#include <cstdint>
#include <memory>
#include <string>
namespace external_bridge {
class ExternalPackageProxy {
    struct State;
    std::unique_ptr<State> state;
public:
    ExternalPackageProxy(uint16_t host_port, const std::wstring& state_root,
                         const std::wstring& nonce, const std::string& broker_token);
    ~ExternalPackageProxy();
    ExternalPackageProxy(const ExternalPackageProxy&) = delete;
    ExternalPackageProxy& operator=(const ExternalPackageProxy&) = delete;
    const std::wstring& family() const;
    const std::wstring& full_name() const;
    const std::string& run_bearer() const;
    const std::wstring& control_root() const;
    uint16_t port() const;
    DWORD process_id() const;
    uint64_t creation_time() const;
    HANDLE process() const;
    std::string evidence() const;
    // Success requires the exact child exit AND actual owned Job Active0.
    bool stop_and_wait(DWORD timeout_ms = 10000);
};
}
