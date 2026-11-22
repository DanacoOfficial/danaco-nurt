open class Animal(val name: String) {
    open fun speak(): String = ""
}

class Dog(name: String) : Animal(name) {
    override fun speak(): String = "woof"
}
