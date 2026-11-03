#pragma once
#include <windows.h>
#include <cstdint>
#include <memory>
#include <string>
#include <vector>

class HostPipeBridge {
    struct State;
    std::unique_ptr<State> state;
public:
    HostPipeBridge(uint16_t target_port, const std::wstring& run_nonce,
                   const std::string& broker_token = "");
    ~HostPipeBridge();
    HostPipeBridge(const HostPipeBridge&) = delete;
    HostPipeBridge& operator=(const HostPipeBridge&) = delete;
    std::vector<HANDLE> inherited_handles() const;
    std::vector<HANDLE> mint_peer_handles(PSID exactSid, DWORD pid, uint64_t birth, HANDLE exactJob);
    void authorize_peer(PSID sid, DWORD process_id, uint64_t creation, HANDLE job);
    HANDLE failure_event() const;
    void stop();
};
