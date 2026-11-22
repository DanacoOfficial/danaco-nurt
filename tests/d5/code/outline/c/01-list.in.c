struct node {
    int value;
    struct node *next;
};

struct node *push(struct node *head, int value);

int length(const struct node *head) {
    int count = 0;
    while (head) {
        count++;
        head = head->next;
    }
    return count;
}
