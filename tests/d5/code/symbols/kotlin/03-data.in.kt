data class Point(val x: Int, val y: Int) {
    fun norm(): Int = x * x + y * y
}
