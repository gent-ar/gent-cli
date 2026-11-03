#pragma once
#include <windows.h>
#include <memory>
#include <string>
class GatewayProxy {
    struct State;
    std::unique_ptr<State> state;
public:
    GatewayProxy(uint16_t host_port, const std::wstring& executable, const std::wstring& state_root, const std::wstring& nonce, const std::wstring& broker_token = L"");
    ~GatewayProxy();
    const std::wstring& moniker() const;
    const std::wstring& runtime() const;
    uint16_t port() const;
    std::string evidence() const;
    HANDLE process() const;
};
int serve_gateway(uint16_t host_port, const wchar_t* status_pipe, const std::string& broker_token = "");
void cleanup_gateway(const std::wstring& moniker, DWORD pid, uint64_t creation, const std::wstring& state_root);
void cleanup_pending_gateway(const std::wstring& state_root, const std::wstring& moniker);
