#pragma once
#include <winsock2.h>
#include <windows.h>
#include <functional>
#include <string>
#include <vector>
namespace external_bridge {
bool serve_package_channels(const std::vector<HANDLE>& channels, HANDLE owner,
    const std::string& bearer, const std::function<bool(uint16_t)>& ready);
}
