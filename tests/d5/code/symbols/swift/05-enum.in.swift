enum Direction {
    case north
    case south

    func opposite() -> Direction {
        return self == .north ? .south : .north
    }
}
