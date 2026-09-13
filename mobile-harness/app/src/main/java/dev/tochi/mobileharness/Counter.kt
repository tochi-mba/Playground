package dev.tochi.mobileharness

/** Pure logic behind the counter screen. Kept free of Android types so it is unit-testable on the JVM. */
object Counter {
    fun next(count: Int): Int = count + 1

    fun label(count: Int): String = "Count: $count"
}
