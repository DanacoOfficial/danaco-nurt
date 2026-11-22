struct Point {
    var x: Int
    var y: Int

    func norm() -> Int {
        return x * x + y * y
    }
}
