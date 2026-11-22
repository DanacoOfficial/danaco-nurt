public record User(String name, int age) {
    public String greeting() {
        return "hello " + name;
    }
}
