public class Repository<T> where T : class
{
    public T Find(int id) => default!;

    public System.Collections.Generic.IEnumerable<T> All()
    {
        yield break;
    }
}
