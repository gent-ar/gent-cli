#pragma once
#include <windows.h>
#include <cstdint>
#include <string>

namespace external_bridge {
// Supplied by the launcher from a reviewed package and an image locked BEFORE
// activation. A readiness file or child-supplied strings cannot create this grant.
struct PackageGrant {
    DWORD pid;
    uint64_t creation;
    DWORD session;
    std::wstring family;
    std::wstring full_name;
    std::wstring appcontainer_sid;
    HANDLE pinned_image;
};

// Returns an owned process handle with QUERY_LIMITED, DUP_HANDLE and SYNCHRONIZE.
// No fallback to an unpackaged process, same-name executable, or guessed SID.
HANDLE capture_package_peer(const PackageGrant& grant);
}
