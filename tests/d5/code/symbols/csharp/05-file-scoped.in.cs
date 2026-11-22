namespace Shop.Api;

public record Order(int Id, decimal Total);

public class OrderService
{
    public Order Find(int id) => new Order(id, 0);
}
