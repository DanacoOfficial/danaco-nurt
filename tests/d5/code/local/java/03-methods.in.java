public class Counter {
    private int value;

    public int bump(int step) {
        value += step;
        return twice(step);
    }

    private int twice(int step) {
        return step * 2;
    }
}
