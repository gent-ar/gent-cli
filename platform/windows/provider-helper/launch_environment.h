#pragma once
// Included after Options and native API declarations in main.cpp.
#include "claurst_gateway_settings.h"
#include "environment_paths.h"
#include "claurst_mcp_environment.h"
static std::vector<wchar_t> environment(Options& o) {
    // Preserve canonical authority/grant strings. Only public tool-facing spelling changes.
    const auto workspace=gent_environment_paths::ordinary(directory_lock(o.workspace).h);
    const auto state=o.state_root.empty()?std::wstring():gent_environment_paths::ordinary(directory_lock(o.state_root).h);
    const auto shell=o.shell.empty()?std::wstring():gent_environment_paths::ordinary(directory_lock(o.shell).h);
    const auto powershell=o.powershell.empty()?std::wstring():gent_environment_paths::ordinary(directory_lock(o.powershell).h);
    const auto launcher=o.powershell_launcher.empty()?std::wstring():gent_environment_paths::ordinary(directory_lock(o.powershell_launcher).h);
    std::wstring windows(32768, L'\0'); UINT n = GetWindowsDirectoryW(windows.data(), static_cast<UINT>(windows.size())); check(n && n < windows.size(), "windows-directory"); windows.resize(n);
    if (o.state_root.empty()) { PWSTR local = nullptr; hresult(SHGetKnownFolderPath(FOLDERID_LocalAppData, 0, nullptr, &local), "local-appdata"); auto held=directory_lock(local); o.env.push_back(L"LOCALAPPDATA=" + gent_environment_paths::ordinary(held.h)); CoTaskMemFree(local); }
    o.env.push_back(L"SystemRoot=" + windows); o.env.push_back(L"windir=" + windows); o.env.push_back(L"SystemDrive=" + windows.substr(0, 2));
    auto shell_path = o.shell.empty() ? L"" : shell.substr(0, shell.find_last_of(L'\\')) + L";";
    if (!o.powershell.empty()) { auto ps = powershell; shell_path = ps.substr(0, ps.find_last_of(L'\\')) + L";" + shell_path; }
    if (!o.powershell_launcher.empty()) { auto launcher_path = launcher; shell_path = launcher_path.substr(0, launcher_path.find_last_of(L'\\')) + L";" + shell_path; o.env.push_back(L"GENT_POWERSHELL_WORKSPACE=" + workspace); o.env.push_back(L"GENT_POWERSHELL_PATH=" + launcher); }
    o.env.push_back(L"PATH=" + shell_path + windows + L"\\System32;" + windows);
    o.env.push_back(L"PATHEXT=.EXE");
    if (!o.powershell.empty()) o.env.push_back(L"DOTNET_SYSTEM_GLOBALIZATION_USENLS=true");
    if (!o.powershell.empty()) { o.env.push_back(L"POWERSHELL_TELEMETRY_OPTOUT=1"); o.env.push_back(L"POWERSHELL_UPDATECHECK=Off"); } o.env.push_back(L"ComSpec=" + (o.gateway_port ? launcher.substr(0, launcher.find_last_of(L'\\')) + L"\\cmd.exe" : windows + L"\\System32\\cmd.exe"));
    if (!o.shell.empty()) { const auto bash = o.gateway_port ? launcher.substr(0, launcher.find_last_of(L'\\')) + L"\\bash.exe" : shell; o.env.push_back(L"CLAUDE_CODE_GIT_BASH_PATH=" + bash); o.env.push_back(L"GENT_GIT_BASH_PATH=" + bash); }
    auto home = state.empty() ? workspace : state + L"\\home";
    o.env.push_back(L"USERPROFILE=" + home); o.env.push_back(L"HOME=" + home); o.env.push_back(L"APPDATA=" + (state.empty() ? home : state + L"\\appdata"));
    if (!o.state_root.empty()) {
        for (const auto& child : {L"home", L"appdata", L"localappdata", L"tmp", L"claude", L"codex"}) { auto path = o.state_root + L"\\" + child;
            check(CreateDirectoryW(path.c_str(), nullptr) || GetLastError() == ERROR_ALREADY_EXISTS, "private-state-directory");
            DWORD attributes = GetFileAttributesW(path.c_str()); check(attributes != INVALID_FILE_ATTRIBUTES && (attributes & FILE_ATTRIBUTE_DIRECTORY) && !(attributes & FILE_ATTRIBUTE_REPARSE_POINT), "private-state-reparse"); }
        o.env.erase(std::remove_if(o.env.begin(), o.env.end(), [](const auto& value) { return value.rfind(L"TMP=", 0) == 0 || value.rfind(L"TMPDIR=", 0) == 0; }), o.env.end());
        o.env.push_back(L"LOCALAPPDATA=" + state + L"\\localappdata");
        o.env.push_back(L"TEMP=" + state + L"\\tmp"); o.env.push_back(L"TMP=" + state + L"\\tmp"); o.env.push_back(L"TMPDIR=" + state + L"\\tmp");
        // Claude uses this explicit override before Windows' AppContainer temp path.
        // The directory is already private to this run and covered by its state grant.
        if (o.gateway_api == L"claude") o.env.push_back(L"CLAUDE_CODE_TMPDIR=" + state + L"\\tmp");
        o.env.push_back(L"CLAUDE_CONFIG_DIR=" + state + L"\\claude"); o.env.push_back(L"CODEX_HOME=" + state + L"\\codex");
    }
    if (o.gateway_port) {
        auto origin = L"http://127.0.0.1:" + std::to_wstring(o.proxy_port);
        if (o.gateway_api == L"claude") { if (!o.powershell.empty() && !o.powershell_launcher.empty()) o.env.push_back(L"CLAUDE_CODE_USE_POWERSHELL_TOOL=1"); o.env.push_back(L"ANTHROPIC_BASE_URL=" + origin); o.env.push_back(L"ANTHROPIC_AUTH_TOKEN=" + o.gateway_token); o.env.push_back(L"GENT_MCP_BRIDGE_TOKEN=" + o.gateway_token); }
        else if (o.gateway_api == L"claurst") {
            o.env.erase(std::remove_if(o.env.begin(), o.env.end(), [](const auto& value) { return value.rfind(L"LLAMA_CPP_HOST=", 0) == 0 || value.rfind(L"CLAURST_HOME=", 0) == 0; }), o.env.end());
            o.env.erase(std::remove_if(o.env.begin(), o.env.end(), [](const auto& value) { return value.rfind(L"CUSTOM_OPENAI_API_KEY=", 0) == 0; }), o.env.end());
            project_claurst_gateway_settings((o.state_root.empty() ? o.workspace : o.state_root + L"\\home") + L"\\.claurst\\settings.json", o.proxy_port);
            o.env.push_back(L"CUSTOM_OPENAI_API_KEY=" + o.gateway_token);
            gent_claurst_mcp_environment::append(o.env, origin, o.gateway_token);
            o.env.push_back(L"CLAURST_HOME=" + home + L"\\.claurst");
        }
        else { o.env.push_back(L"GENT_PROVIDER_GATEWAY_TOKEN=" + o.gateway_token);
            for (const auto& value : std::vector<std::wstring>{L"model_provider=\"gent_gateway\"", L"model_providers.gent_gateway.name=\"Gent isolated gateway\"",
                L"model_providers.gent_gateway.base_url=\"" + origin + L"/v1\"", L"model_providers.gent_gateway.env_key=\"GENT_PROVIDER_GATEWAY_TOKEN\"",
                L"model_providers.gent_gateway.wire_api=\"responses\"", L"model_providers.gent_gateway.requires_openai_auth=false", L"model_providers.gent_gateway.supports_websockets=false"}) {
                o.args.push_back(L"-c"); o.args.push_back(value);
            }
        }
    }
    DWORD drives = GetLogicalDrives(); check(drives != 0, "logical-drives");
    for (unsigned i = 0; i < 26; ++i) if (drives & (1u << i)) { auto c = static_cast<wchar_t>(L'A' + i); o.env.push_back(std::wstring(L"=") + c + L":=" + c + L":\\"); }
    std::sort(o.env.begin(), o.env.end(), [](const auto& a, const auto& b) { return _wcsicmp(a.c_str(), b.c_str()) < 0; });
    std::vector<wchar_t> result; for (const auto& entry : o.env) { result.insert(result.end(), entry.begin(), entry.end()); result.push_back(0); } result.push_back(0); return result;
}
