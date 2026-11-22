class Box<T>(private val value: T) {
    fun <R> map(f: (T) -> R): Box<R> = Box(f(value))
}

typealias Names = List<String>
