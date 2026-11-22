import kotlin.math.sqrt

fun area(r: Int): Int {
    return 3 * r * r
}

fun main() {
    val r = area(2)
    println(sqrt(r.toDouble()))
}
