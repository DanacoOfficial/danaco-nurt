public class User
{
    public string Name { get; set; }
    private int age;
    public const int Max = 10;
    public event System.EventHandler Changed;
}
