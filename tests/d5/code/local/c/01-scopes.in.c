#include <stdio.h>

static int area(int r) {
    return 3 * r * r;
}

int main(void) {
    int r = area(2);
    printf("%d\n", r);
    return 0;
}
