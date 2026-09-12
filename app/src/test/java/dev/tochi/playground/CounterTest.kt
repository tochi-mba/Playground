package dev.tochi.playground

import org.junit.Assert.assertEquals
import org.junit.Test

class CounterTest {
    @Test
    fun nextIncrementsByOne() {
        assertEquals(1, Counter.next(0))
        assertEquals(6, Counter.next(5))
    }

    @Test
    fun labelFormatsCount() {
        assertEquals("Count: 3", Counter.label(3))
    }
}
