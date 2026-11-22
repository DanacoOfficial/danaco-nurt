template <typename T>
T max_of(T a, T b) {
    return a > b ? a : b;
}

template <typename T>
class Stack {
public:
    void push(T value);
    int size() const;
};
