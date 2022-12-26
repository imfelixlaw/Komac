package data.installer

import Errors
import Validation
import com.github.ajalt.mordant.rendering.TextColors.brightWhite
import com.github.ajalt.mordant.rendering.TextColors.brightYellow
import com.github.ajalt.mordant.rendering.TextColors.gray
import com.github.ajalt.mordant.rendering.TextColors.red
import com.github.ajalt.mordant.terminal.Terminal
import data.InstallerManifestData
import data.SharedManifestData
import input.PromptType
import input.Prompts
import input.YamlExtensions.convertToYamlList
import org.koin.core.component.KoinComponent
import org.koin.core.component.get
import org.koin.core.component.inject
import schemas.InstallerSchema
import schemas.SchemasImpl

object Protocols : KoinComponent {
    private val installerManifestData: InstallerManifestData by inject()
    private val schemasImpl: SchemasImpl by inject()
    private val sharedManifestData: SharedManifestData by inject()
    private val protocolsSchema = schemasImpl.installerSchema.definitions.protocols

    suspend fun Terminal.protocolsPrompt() {
        do {
            println(
                brightYellow("${Prompts.optional} ${protocolsSchema.description} (Max ${protocolsSchema.maxItems})")
            )
            val input = prompt(
                prompt = brightWhite(PromptType.Protocols.toString()),
                default = getPreviousValue()?.joinToString(", ")?.also { println(gray("Previous protocols: $it")) }
            )?.trim()?.convertToYamlList(protocolsSchema.uniqueItems)
            val (protocolsValid, error) = areProtocolsValid(input)
            if (protocolsValid == Validation.Success) installerManifestData.protocols = input
            error?.let { println(red(it)) }
            println()
        } while (protocolsValid != Validation.Success)
    }

    fun areProtocolsValid(
        protocols: Iterable<String>?,
        installerSchema: InstallerSchema = get<SchemasImpl>().installerSchema
    ): Pair<Validation, String?> {
        val protocolsSchema = installerSchema.definitions.protocols
        return when {
            (protocols?.count() ?: 0) > protocolsSchema.maxItems -> {
                Validation.InvalidLength to Errors.invalidLength(max = protocolsSchema.maxItems)
            }
            protocols?.any { it.length > protocolsSchema.items.maxLength } == true -> {
                Validation.InvalidLength to Errors.invalidLength(
                    max = protocolsSchema.items.maxLength,
                    items = protocols.filter { it.length > protocolsSchema.items.maxLength }
                )
            }
            else -> Validation.Success to null
        }
    }

    private suspend fun getPreviousValue(): List<String>? {
        return sharedManifestData.remoteInstallerData.await().let {
            it?.protocols ?: it?.installers?.get(installerManifestData.installers.size)?.protocols
        }
    }
}
