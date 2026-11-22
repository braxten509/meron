// The Android app generates resources and notification text from the same three catalogs.
// XML checks cover the default language and Android's Brazilian Portuguese qualifier.
// ICU checks cover saved language tags, regional plurals and the English fallback.
package jp.nonbili.meron

import org.junit.Assert.assertEquals
import org.junit.Test
import java.io.File
import java.util.Locale

class GeneratedLocalizationTest {
    @Test
    fun generatedAndroidXmlContainsTheDefaultEnglishStrings() {
        assertEquals(
            "Cancel",
            generatedStringValue("src/main/res/values/strings.xml", "buttons_cancel"),
        )
    }

    @Test
    fun generatedAndroidXmlUsesSpecialBrazilianPortugueseResourceFolder() {
        assertEquals(
            "Cancelar",
            generatedStringValue("src/main/res/values-pt-rBR/strings.xml", "buttons_cancel"),
        )
    }

    @Test
    fun generatedIcuHelperContainsPluralMessagesForSupportedLocales() {
        assertEquals("1 file", generatedIcuString("en", "chat.fileItems", mapOf("count" to 1)))
        assertEquals("2 files", generatedIcuString("en", "chat.fileItems", mapOf("count" to 2)))
        assertEquals(setOf("en", "pt_BR", "ru"), generatedIcuStrings.keys)
        for (locale in listOf("pt-BR", "pt_BR", "pt")) {
            assertEquals("1 nova mensagem", generatedIcuString(locale, "notify.newMessageCount", mapOf("count" to 1)))
            assertEquals("3 novas mensagens", generatedIcuString(locale, "notify.newMessageCount", mapOf("count" to 3)))
        }
        assertEquals("2 files", generatedIcuString("ja", "chat.fileItems", mapOf("count" to 2)))
    }

    @Test
    fun inlinePluralRetainsTheDestinationAndSurroundingText() {
        val args = mapOf("count" to 2, "name" to "Work")
        assertEquals("Moved 2 conversations to Work", generatedIcuString("en", "meronFolders.moved", args))
        assertEquals("2 conversas movidas para Work", generatedIcuString("pt-BR", "meronFolders.moved", args))
        assertEquals("Перемещено в Work: 2 переписки", generatedIcuString("ru", "meronFolders.moved", args))
    }

    @Test
    fun russianDeviceLocalesResolveToTheRussianCatalog() {
        assertEquals("ru", catalogLanguageTag(Locale.forLanguageTag("ru")))
        assertEquals("ru", catalogLanguageTag(Locale.forLanguageTag("ru-RU")))
    }

    @Test
    fun russianResourcesAndNotificationsUseRussianPluralForms() {
        assertEquals("Отмена", generatedStringValue("src/main/res/values-ru/strings.xml", "buttons_cancel"))
        val endings =
            mapOf(
                0 to "новых писем",
                1 to "новое письмо",
                2 to "новых письма",
                4 to "новых письма",
                5 to "новых писем",
                11 to "новых писем",
                12 to "новых писем",
                14 to "новых писем",
                21 to "новое письмо",
                22 to "новых письма",
                25 to "новых писем",
                111 to "новых писем",
            )
        for ((count, ending) in endings) {
            for (locale in listOf("ru", "ru-RU", "ru_RU")) {
                assertEquals(
                    "$count $ending",
                    generatedIcuString(locale, "notify.newMessageCount", mapOf("count" to count)),
                )
            }
        }
    }

    private fun generatedStringValue(
        resourcePath: String,
        name: String,
    ): String {
        val xml = File(resourcePath).readText()
        val regex = Regex("""<string name="$name">(.+)</string>""")
        return regex.find(xml)?.groupValues?.get(1)
            ?: error("Missing $name in $resourcePath")
    }
}
