#pragma once
#include <algorithm>
#include <cstdint>
#include <limits>

namespace contained_stream {
// Progress can renew idle time, never the total response budget. Initial model
// prefill may use the total budget before the first transport data arrives.
class Budget {
    uint64_t total_end;
    uint64_t idle_end;
    uint64_t idle_ms;
    static uint64_t add(uint64_t now, uint64_t span) {
        return span > std::numeric_limits<uint64_t>::max() - now
            ? std::numeric_limits<uint64_t>::max() : now + span;
    }
public:
    explicit Budget(uint64_t now, uint64_t total = 300000, uint64_t idle = 120000)
        : total_end(add(now, total)), idle_end(total_end), idle_ms(idle) {}
    uint64_t deadline() const { return std::min(total_end, idle_end); }
    bool expired(uint64_t now) const { return now >= deadline(); }
    bool progress(uint64_t now) {
        if (expired(now)) return false;
        idle_end = std::min(total_end, add(now, idle_ms));
        return true;
    }
};
}
