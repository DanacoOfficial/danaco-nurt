public class Shadow
{
    private int value = 1;

    public int Apply(int value)
    {
        return value + this.value;
    }

    public int Sum(int[] items)
    {
        var total = 0;
        foreach (var item in items)
        {
            total += item;
        }
        return total;
    }
}
