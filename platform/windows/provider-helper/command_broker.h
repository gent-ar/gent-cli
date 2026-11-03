#pragma once
#include <windows.h>
#include <memory>
#include <string>
#include <vector>
struct CommandBrokerConfig {
    HANDLE run_job=nullptr,provider_token=nullptr;
    std::wstring helper_image,state_root,ordinary_state,workspace,bash,powershell,shim_directory,bearer,gateway_origin,node_directory,ordinary_workspace;
    std::vector<std::wstring> read,write;
};
class CommandBroker {
public:
    explicit CommandBroker(CommandBrokerConfig config);
    ~CommandBroker();
    CommandBroker(const CommandBroker&)=delete;
    CommandBroker& operator=(const CommandBroker&)=delete;
    bool stop() noexcept;
    std::string accounting();
private:
    struct State;
    std::unique_ptr<State> state;
};
