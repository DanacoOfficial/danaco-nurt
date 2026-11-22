public delegate void Handler(int code);

public class Service
{
    public Service(Handler handler) {}

    ~Service() {}
}
