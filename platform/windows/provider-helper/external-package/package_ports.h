#pragma once
#include <cstdint>
namespace external_bridge {
// Must exactly match the reviewed package firewall declaration. No dynamic
// firewall mutation, wildcard allocator or name-based process reclamation.
constexpr uint16_t package_port_first = 55490;
constexpr uint16_t package_port_last = 55521;
inline bool package_port_allowed(unsigned port) {
    return port >= package_port_first && port <= package_port_last;
}
}
