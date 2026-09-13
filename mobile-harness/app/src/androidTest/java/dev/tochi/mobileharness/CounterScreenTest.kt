package dev.tochi.mobileharness

import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class CounterScreenTest {
    @get:Rule
    val composeRule = createAndroidComposeRule<MainActivity>()

    @Test
    fun tappingIncrementUpdatesCount() {
        composeRule.onNodeWithTag("count").assertTextEquals("Count: 0")
        composeRule.onNodeWithTag("increment").performClick()
        composeRule.onNodeWithTag("count").assertTextEquals("Count: 1")
    }
}
