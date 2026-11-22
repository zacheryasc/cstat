#ifndef CPP_CONTRACT_API_HPP
#define CPP_CONTRACT_API_HPP

#include "cycle.hpp"
#define DECLARE(name) int name(int value)

namespace demo {
struct Widget {
    explicit Widget(int value);
    ~Widget();
    Widget(const Widget&) = delete;
    int operator()(int value) const;
    int operator()(double value) const;
    explicit operator bool() const;
private:
    int bias_;
};

DECLARE(generated);
using Count = unsigned;
enum class State { off, on };

int (*select())(int);
extern int (*callback)(int);

template <typename T>
concept Addable = requires(T value) { value + value; };

template <typename T> requires Addable<T>
T twice(T value) { return value + value; }

template <>
int twice<int>(int value) { return value * 2; }
}
#endif
