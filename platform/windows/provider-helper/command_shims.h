#pragma once
static std::vector<Handle> lock_command_shims(const std::wstring& launcher, const std::wstring& digest) {
    const auto parent = launcher.substr(0, launcher.find_last_of(L'\\'));
    std::vector<Handle> locks;
    for (const auto* name : {L"bash.exe", L"cmd.exe"}) {
        const auto path = parent + L"\\" + name;
        Handle shim(CreateFileW(path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_FLAG_SEQUENTIAL_SCAN, nullptr));
        check(shim.h != INVALID_HANDLE_VALUE, "command-shim-lock"); check(hash(shim.h) == digest, "command-shim-digest");
        locks.push_back(std::move(shim));
    }
    return locks;
}
static CommandBrokerConfig command_broker_config(const Options& o, HANDLE job, HANDLE token, const std::wstring& self) {
    CommandBrokerConfig config; config.run_job = job; config.provider_token = token;
    const auto ordinary = [](const std::wstring& path) { auto held = directory_lock(path); return gent_environment_paths::ordinary(held.h); };
    config.helper_image = ordinary(self); config.state_root = o.state_root; config.ordinary_state = ordinary(o.state_root); config.workspace = o.workspace; config.ordinary_workspace = ordinary(o.workspace);
    config.bash = ordinary(o.shell); config.powershell = ordinary(o.powershell);
    const auto launcher = ordinary(o.powershell_launcher); config.shim_directory = launcher.substr(0, launcher.find_last_of(L'\\'));
    config.bearer = o.gateway_token; config.gateway_origin = L"http://127.0.0.1:" + std::to_wstring(o.proxy_port);
    const auto runtime_root = config.helper_image.substr(0, config.helper_image.find_last_of(L'\\'));
    const auto node_image = runtime_root.substr(0, runtime_root.find_last_of(L'\\')) + L"\\node\\bin\\node.exe";
    const auto node_attributes = GetFileAttributesW(node_image.c_str());
    if (node_attributes != INVALID_FILE_ATTRIBUTES && !(node_attributes & FILE_ATTRIBUTE_DIRECTORY)) config.node_directory = node_image.substr(0, node_image.find_last_of(L'\\'));
    config.read = o.read; config.write = o.write; return config;
}
