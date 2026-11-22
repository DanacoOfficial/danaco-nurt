#ifndef API_H
#define API_H

int api_open(const char *path);
void api_close(int fd);
extern int api_errno;

#endif
