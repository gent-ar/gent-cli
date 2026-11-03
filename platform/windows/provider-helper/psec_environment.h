#pragma once
#include <windows.h>
#include <algorithm>
#include <cstdint>
#include <cstring>
#include <stdexcept>
#include <string>
#include <vector>
namespace gent_psec {
inline void require(bool ok, const char* label) {
    if (!ok) { const auto error = GetLastError(); throw std::runtime_error(std::string(label) + ":" + std::to_string(error)); }
}
inline void require_hresult(HRESULT value, const char* label) {
    if (FAILED(value)) throw std::runtime_error(std::string(label) + ":" + std::to_string(static_cast<uint32_t>(value)));
}
inline std::string utf8(const std::wstring& value) {
    if (value.empty()) return {};
    int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(), static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
    require(size > 0, "unicode"); std::string text(size, '\0');
    WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(), static_cast<int>(value.size()), text.data(), size, nullptr, nullptr); return text;
}
inline std::wstring normalized_path(std::wstring path) {
    if (path.rfind(L"\\\\?\\", 0) == 0) path.erase(0, 4);
    for (auto& c : path) if (c == L'/') c = L'\\';
    while (path.size() > 3 && path.back() == L'\\') path.pop_back(); return path;
}
inline bool path_within(const std::wstring& path, const std::wstring& root) {
    return path.size() >= root.size() && _wcsnicmp(path.c_str(), root.c_str(), root.size()) == 0
        && (path.size() == root.size() || path[root.size()] == L'\\');
}
enum class Network { Disabled, ProviderGateway, DirectInternet };
struct Policy {
    std::vector<std::wstring> read, write, nested_read_only;
    Network network = Network::Disabled;
    std::wstring gateway_peer;
    uint16_t proxy_port = 0;
};
class Spec {
public:
    std::vector<uint8_t> bytes;
    explicit Spec(const Policy& policy) : bytes(72, 0) {
        u32(0, static_cast<uint32_t>(root)); std::memcpy(bytes.data() + 4, "PSEC", 4); u16(8, 22); u16(10, 40); u32(root, static_cast<uint32_t>(root - 8));
        u16(12, 4); u16(root + 4, 1); u16(root + 6, 1);
        if (policy.network == Network::ProviderGateway) string(set(root, 1), L"privateNetworkClientServer");
        if (policy.network == Network::DirectInternet) string(set(root, 1), L"internetClient");
        paths(4, policy.write);
        std::vector<std::wstring> read_directories, read_only;
        for (const auto& path : policy.read) {
            const DWORD attrs = GetFileAttributesW(path.c_str());
            if (attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_DIRECTORY)) read_directories.push_back(normalized_path(path));
        }
        for (const auto& path : policy.read) {
            const auto normalized = normalized_path(path);
            const bool writable = std::any_of(policy.write.begin(), policy.write.end(), [&](const auto& root) { return path_within(normalized, normalized_path(root)); });
            const bool nested = std::any_of(read_directories.begin(), read_directories.end(), [&](const auto& root) { return root.size() < normalized.size() && path_within(normalized, root); });
            if (!writable && !nested) read_only.push_back(path);
        }
        read_only.insert(read_only.end(), policy.nested_read_only.begin(), policy.nested_read_only.end());
        paths(5, read_only);
        std::vector<std::wstring> enumerate;
        for (const auto& path : read_only) {
            DWORD attrs = GetFileAttributesW(path.c_str());
            if (attrs != INVALID_FILE_ATTRIBUTES && !(attrs & FILE_ATTRIBUTE_DIRECTORY)) {
                auto parent = path.substr(0, path.find_last_of(L"\\/"));
                if (std::find(enumerate.begin(), enumerate.end(), parent) == enumerate.end()) enumerate.push_back(parent);
            }
        }
        paths(8, enumerate);
        std::vector<std::wstring> denied; DWORD drives = GetLogicalDrives(); require(drives != 0, "logical-drives");
        for (unsigned i = 0; i < 26; ++i) if (drives & (1u << i)) denied.push_back(std::wstring(1, static_cast<wchar_t>(L'A' + i)) + L":\\");
        paths(6, denied);
        u16(12 + 7 * 2, 32); size_t network = table(4); offset(root + 32, network);
        if (policy.network == Network::ProviderGateway) {
            string(set(network, 2), policy.gateway_peer);
            size_t proxy_field = set(network, 0), proxy = table(1); offset(proxy_field, proxy); string(set(proxy, 0), L"http://127.0.0.1:" + std::to_wstring(policy.proxy_port));
        } else {
            size_t egress_field = set(network, 1), egress = table(3); offset(egress_field, egress);
            if (policy.network == Network::DirectInternet) bytes[set(egress, 0)] = 1;
        }
    }
private:
    static constexpr size_t root = 32;
    void u16(size_t p, uint16_t value) { std::memcpy(bytes.data() + p, &value, 2); }
    void u32(size_t p, uint32_t value) { std::memcpy(bytes.data() + p, &value, 4); }
    void align() { while (bytes.size() % 4) bytes.push_back(0); }
    void offset(size_t field, size_t target) { u32(field, static_cast<uint32_t>(target - field)); }
    size_t set(size_t object, size_t slot) { uint32_t distance = 0; std::memcpy(&distance, bytes.data() + object, 4);
        u16(object - distance + 4 + slot * 2, static_cast<uint16_t>(4 + slot * 4)); return object + 4 + slot * 4; }
    void string(size_t field, const std::wstring& value) { auto text = utf8(value); align(); size_t p = bytes.size(); bytes.resize(p + 4 + text.size() + 1);
        offset(field, p); u32(p, static_cast<uint32_t>(text.size())); std::memcpy(bytes.data() + p + 4, text.data(), text.size()); }
    void paths(size_t slot, const std::vector<std::wstring>& values) {
        if (values.empty()) return; u16(12 + slot * 2, static_cast<uint16_t>(4 + slot * 4)); align(); size_t p = bytes.size(); bytes.resize(p + 4 + values.size() * 4);
        offset(root + 4 + slot * 4, p); u32(p, static_cast<uint32_t>(values.size())); for (size_t i = 0; i < values.size(); ++i) string(p + 4 + i * 4, values[i]); }
    size_t table(size_t slots) { align(); size_t vtable = bytes.size(); bytes.resize(vtable + 4 + slots * 2); align(); size_t object = bytes.size(); bytes.resize(object + 4 + slots * 4);
        u16(vtable, static_cast<uint16_t>(4 + slots * 2)); u16(vtable + 2, static_cast<uint16_t>(4 + slots * 4)); u32(object, static_cast<uint32_t>(object - vtable)); return object; }
};
using CreateEnvironment = HRESULT(WINAPI*)(const BYTE*, DWORD, DWORD, HANDLE*);
using CloseEnvironment = void(WINAPI*)(HANDLE);
using QuerySupport = HRESULT(WINAPI*)(uint64_t*);
using QueryVersion = HRESULT(WINAPI*)(DWORD, BOOLEAN*, DWORD*);
struct Api {
    CreateEnvironment create; CloseEnvironment close; QuerySupport query; QueryVersion version;
    Api() { auto dll = LoadLibraryExW(L"processmodel.dll", nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32); require(dll != nullptr, "psec-unavailable");
        create = reinterpret_cast<CreateEnvironment>(GetProcAddress(dll, "CreateProcessSecurityEnvironment")); close = reinterpret_cast<CloseEnvironment>(GetProcAddress(dll, "CloseProcessSecurityEnvironment"));
        query = reinterpret_cast<QuerySupport>(GetProcAddress(dll, "QueryProcessSecurityEnvironmentSupport")); version = reinterpret_cast<QueryVersion>(GetProcAddress(dll, "IsProcessSecurityEnvironmentVersionSupported"));
        require(create && close && query && version, "psec-exports-unavailable"); BOOLEAN available = FALSE; DWORD minor = 0; require_hresult(version(1, &available, &minor), "psec-version-query"); require(available != FALSE, "psec-version-unavailable"); }
};
struct Environment {
    Api& api; HANDLE value = nullptr;
    Environment(Api& owner, const Spec& spec) : api(owner) {
        require_hresult(api.create(spec.bytes.data(), static_cast<DWORD>(spec.bytes.size()), 1, &value), "psec-create");
        require(value != nullptr, "psec-null");
    }
    Environment(const Environment&) = delete;
    Environment& operator=(const Environment&) = delete;
    ~Environment() { if (value) api.close(value); }
};
constexpr DWORD_PTR security_environment_attribute = 35 | 0x00020000;
}
