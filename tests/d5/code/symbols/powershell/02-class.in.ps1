class Person {
    [string]$Name

    Person([string]$name) {
        $this.Name = $name
    }

    [string] Greet() {
        return "hi"
    }
}
