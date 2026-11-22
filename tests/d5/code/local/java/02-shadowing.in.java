public class Shadow {
    private int value = 1;

    int shadow(int value) {
        return value + this.value;
    }

    int loop(int[] items) {
        int total = 0;
        for (int item : items) {
            total += item;
        }
        return total;
    }
}
