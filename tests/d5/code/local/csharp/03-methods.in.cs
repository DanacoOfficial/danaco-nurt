public class Counter
{
    private int value;

    public int Bump(int step)
    {
        value += step;
        return Twice(step);
    }

    private int Twice(int step) => step * 2;
}
