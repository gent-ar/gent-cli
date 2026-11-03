#include <cstddef>
#include <cstdint>
#include <limits>
#include <stdexcept>
#include <iostream>
#include "model_protocol_bounds.h"
using namespace gent_model_protocol;
static unsigned checks=0;
static void yes(bool value){if(!value)throw std::runtime_error("bounds-test");++checks;}
template<class F> static void rejects(F f){bool bad=false;try{f();}catch(const std::runtime_error&){bad=true;}yes(bad);}
int main(){
 yes(required(-16384,16384)==16384);yes(required(-1,16384)==1);
 rejects([]{required(-16385,16384);});rejects([]{required(std::numeric_limits<int32_t>::min(),16384);});
 rejects([]{required(0,16384);});rejects([]{required(1,16384);});
 yes(required(-1024,1024)==1024);rejects([]{required(-1025,1024);});rejects([]{required(std::numeric_limits<int32_t>::min(),1024);});
 piece_size(0,0);++checks;piece_size(16384,16384);++checks;
 rejects([]{piece_size(16385,16384);});rejects([]{piece_size(-1,16384);});rejects([]{piece_size(std::numeric_limits<int32_t>::min(),16384);});
 std::cout<<checks<<" PASS\n";
}
