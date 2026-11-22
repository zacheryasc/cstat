#include "api.hpp"
#include <utility>
#define OPTIONAL_HEADER "optional.hpp"
#include OPTIONAL_HEADER

namespace demo {
Widget::Widget(int value) : bias_(value > 0 ? value : 0) {}
Widget::~Widget() = default;

#ifdef WIDE_BUILD
int configured(long value) { return value > 0; }
#else
int configured(int value) { return value != 0; }
#endif

int Widget::operator()(int value) const {
    auto inner = [this](int n) {
        if (n < 0) return bias_;
        auto nested = [](int m) { return m ? 1 : 0; };
        return nested(n);
    };
    const char* text = R"tag(// not a comment
/* neither is this */)tag";
    if (value > 0 && text[0]) {
        for (int i = 0; i < value; ++i) {
            if (i == bias_) break;
        }
    }
    switch (value) {
        case 0:
        case 1: return inner(value);
        default: break;
    }
    try { throw value; } catch (int e) { return e || bias_; }
    return 0;
}

int Widget::operator()(double value) const { return value > bias_; }
}
