import java.util.function.Function;

public class Box<T> {
    private T value;

    public <U> Box<U> map(Function<T, U> f) {
        return null;
    }
}
