#include <stdlib.h>
#include <string.h>

char *copy(const char *text) {
    char *out = malloc(strlen(text) + 1);
    strcpy(out, text);
    return out;
}
