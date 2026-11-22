using System;

public static class Geometry
{
    static int Area(int r)
    {
        return 3 * r * r;
    }

    public static void Main()
    {
        var r = Area(2);
        Console.WriteLine(r);
    }
}
