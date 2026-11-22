class Config private constructor(val name: String) {
    companion object {
        const val MAX = 10

        fun create(): Config = Config("default")
    }
}
