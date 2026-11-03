#pragma once
#include "external_channel_client.h"
#include <set>
#include <vector>
namespace external_bridge {
inline std::vector<HANDLE> admit_package_channels(const std::string& descriptor,
    const ClientBinding& binding, HANDLE owner) {
    if (descriptor.empty() || descriptor.size() > 1024 || descriptor.back() != '\n') return {};
    std::istringstream input(descriptor);
    std::string marker, nonce, extra;
    uint64_t pid = 0, birth = 0, owner_pid = 0, owner_birth = 0, session = 0, count = 0;
    if (!(input >> marker >> pid >> birth >> owner_pid >> owner_birth >> session >> nonce >> count)
        || marker != "GENT-BRIDGE-DESCRIPTOR/2" || nonce != binding.nonce || count != 9
        || pid != GetCurrentProcessId() || birth != process_birth(GetCurrentProcess())
        || owner_pid != binding.owner_pid || owner_birth != binding.owner_birth
        || session != binding.session || GetProcessId(owner) != binding.owner_pid
        || process_birth(owner) != binding.owner_birth
        || WaitForSingleObject(owner, 0) != WAIT_TIMEOUT) return {};
    DWORD own_session = 0;
    if (!ProcessIdToSessionId(GetCurrentProcessId(), &own_session) || own_session != session) return {};
    std::set<uint64_t> unique; std::vector<HANDLE> result;
    for (uint64_t index = 0; index < count; ++index) {
        uint64_t raw = 0;
        if (!(input >> raw) || !raw || raw > UINTPTR_MAX || !unique.insert(raw).second) return {};
        const auto handle = reinterpret_cast<HANDLE>(static_cast<uintptr_t>(raw));
        if (!checked_pipe(handle, binding.owner_pid, binding.session)) return {};
        result.push_back(handle);
    }
    if (input >> extra) return {};
    for (const auto handle : result) if (!SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0)) return {};
    return result;
}
}
