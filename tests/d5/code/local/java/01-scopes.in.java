import java.util.List;

public class Geometry {
    static int area(int r) {
        return 3 * r * r;
    }

    public static void main(String[] args) {
        int r = area(2);
        List<Integer> values = List.of(r);
        System.out.println(values);
    }
}
