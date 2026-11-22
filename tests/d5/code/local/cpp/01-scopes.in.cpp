#include <iostream>

int area(int r) {
    return 3 * r * r;
}

int main() {
    int r = area(2);
    std::cout << r << std::endl;
    return 0;
}
