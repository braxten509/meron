// Renders contact directory, search, notes and mail
// history on mobile. A remembered screen state keeps the
// selected correspondent associated with the displayed
// entries.

package jp.nonbili.meron.ui

import androidx.compose.animation.core.animate
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Sort
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Contacts
import androidx.compose.material.icons.filled.ErrorOutline
import androidx.compose.material.icons.filled.PushPin
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.backhandler.BackHandler
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import jp.nonbili.meron.shared.AccountSummary
import jp.nonbili.meron.shared.ContactSuggestion
import jp.nonbili.meron.shared.FolderSummary
import jp.nonbili.meron.shared.MeronCore
import jp.nonbili.meron.shared.ThreadSummary
import jp.nonbili.meron.shared.formatContactSuggestion
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

@Composable
internal fun rememberContactsScreenState(core: MeronCore): ContactsScreenState = remember(core) { ContactsScreenState(core) }

/** Local directory; it never changes the mailbox's current query or folder. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalComposeUiApi::class)
@Composable
internal fun ContactsScreen(
    core: MeronCore,
    onBack: () -> Unit,
    onCompose: (String) -> Unit,
    onOpenThread: (ThreadSummary) -> Unit,
    onCopy: (String, String) -> Unit,
    initialContact: ContactSuggestion? = null,
    canCompose: Boolean = true,
    state: ContactsScreenState = rememberContactsScreenState(core),
    onInitialContactConsumed: () -> Unit = {},
    active: Boolean = true,
    accounts: List<AccountSummary> = emptyList(),
    foldersByAccount: Map<String, List<FolderSummary>> = emptyMap(),
    showSenderImages: Boolean = true,
) {
    val scope = rememberCoroutineScope()
    val query = state.query
    val sort = state.sort
    val limit = state.limit
    var retry by remember { mutableStateOf(0) }
    var sortOpen by remember { mutableStateOf(false) }
    val selected = state.selected
    val goBack = { if (state.selected != null) state.select(null) else onBack() }
    val colors = MaterialTheme.colorScheme
    BackHandler(enabled = active && selected != null) { state.select(null) }
    LaunchedEffect(core) { state.loadPins() }
    LaunchedEffect(initialContact) {
        if (initialContact != null) {
            if (contactAddressKey(initialContact.addr) != state.selected?.addr) state.select(initialContact)
            onInitialContactConsumed()
        }
    }
    LaunchedEffect(query, sort, limit, retry) {
        delay(150)
        state.loadDirectory(query, sort, limit)
    }
    LaunchedEffect(selected?.addr) {
        if (selected != null) {
            launch { state.loadHistory() }
            launch { state.loadNote(selected.addr) }
        }
    }
    Scaffold(
        containerColor = colors.surface,
        topBar = {
            Column(Modifier.background(colors.surface)) {
                TopAppBar(
                    title = {
                        Text(
                            tr("contacts.title"),
                            fontSize = 18.sp,
                            fontWeight = FontWeight.SemiBold,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    },
                    colors = TopAppBarDefaults.topAppBarColors(containerColor = colors.surface),
                    navigationIcon = {
                        IconButton(onClick = goBack) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = tr("buttons.back"))
                        }
                    },
                    actions = {
                        if (selected != null) {
                            val pinned = state.isPinned(selected)
                            val pinLabel = tr(if (pinned) "contacts.unpin" else "contacts.pin")
                            IconButton(
                                onClick = { scope.launch { state.togglePin(selected) } },
                                enabled = state.pinsLoaded && !state.pinsSaving,
                            ) {
                                Icon(
                                    if (pinned) Icons.Filled.PushPin else Icons.Outlined.PushPin,
                                    contentDescription = pinLabel,
                                    tint = if (pinned) colors.primary else colors.onSurfaceVariant,
                                )
                            }
                        }
                    },
                )
                if (selected == null) {
                    ContactsSearchBar(
                        query = query,
                        onQueryChange = {
                            state.query = it
                            state.limit = 100
                        },
                        sort = sort,
                        sortOpen = sortOpen,
                        onSortOpenChange = { sortOpen = it },
                        sortEnabled = state.pinsLoaded && !state.sortSaving,
                        onSelectSort = { option ->
                            scope.launch { state.setSort(option) }
                            sortOpen = false
                        },
                    )
                }
                HorizontalDivider(color = colors.outlineVariant.copy(alpha = 0.5f))
            }
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .consumeWindowInsets(padding)
                .imePadding(),
        ) {
            if (state.pinsError) {
                ContactFailure(
                    onRetry = {
                        scope.launch {
                            if (!state.pinsLoaded) {
                                state.loadPins()
                            } else {
                                state.retryPin()
                            }
                        }
                    },
                )
            }
            if (state.sortError) ContactFailure { scope.launch { state.retrySort() } }
            if (selected == null) {
                val matchingPins =
                    state.pins
                        .filter {
                            "${it.name} ${it.addr}".contains(query.trim(), ignoreCase = true)
                        }.sortedBy { it.name.ifBlank { it.addr }.lowercase() }
                val others = state.contacts.filterNot(state::isPinned)
                val directoryListState = rememberLazyListState()
                LazyColumn(
                    Modifier.fillMaxSize().appScrollbar(directoryListState),
                    state = directoryListState,
                    contentPadding = PaddingValues(vertical = 6.dp),
                ) {
                    if (matchingPins.isNotEmpty()) {
                        item(key = "section:pinned") { ContactSectionTitle(tr("contacts.pinned")) }
                        items(matchingPins, key = { "pin:${contactAddressKey(it.addr)}" }) { contact ->
                            ContactDirectoryRow(
                                state = state,
                                contact = contact,
                                showSenderImages = showSenderImages,
                                onSelect = { state.select(contact) },
                                onPin = { scope.launch { state.togglePin(contact) } },
                            )
                        }
                        if (others.isNotEmpty()) {
                            item(key = "section:other") { ContactSectionTitle(tr("contacts.other")) }
                        }
                    }
                    items(others, key = { contactAddressKey(it.addr) }) { contact ->
                        ContactDirectoryRow(
                            state = state,
                            contact = contact,
                            showSenderImages = showSenderImages,
                            onSelect = { state.select(contact) },
                            onPin = { scope.launch { state.togglePin(contact) } },
                        )
                    }
                    if (state.directoryLoading) item(key = "loading") { ContactLoading() }
                    if (state.directoryError) item(key = "error") { ContactFailure { retry++ } }
                    if (!state.directoryLoading && !state.directoryError && matchingPins.isEmpty() && others.isEmpty()) {
                        item(key = "empty") {
                            Column(
                                Modifier.fillMaxWidth().padding(horizontal = 24.dp, vertical = 48.dp),
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.spacedBy(10.dp),
                            ) {
                                Icon(
                                    Icons.Filled.Contacts,
                                    contentDescription = null,
                                    modifier = Modifier.size(36.dp),
                                    tint = colors.onSurfaceVariant.copy(alpha = 0.55f),
                                )
                                Text(
                                    tr("contacts.empty"),
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = colors.onSurfaceVariant,
                                )
                            }
                        }
                    }
                    if (!state.directoryError && state.contacts.size >= limit) {
                        item(key = "more") {
                            Box(Modifier.fillMaxWidth().padding(vertical = 12.dp), contentAlignment = Alignment.Center) {
                                OutlinedButton(
                                    onClick = { state.limit += 100 },
                                    enabled = !state.directoryLoading,
                                ) {
                                    Text(tr("threads.actions.loadMore"))
                                }
                            }
                        }
                    }
                }
            } else {
                ContactDetails(
                    state = state,
                    contact = selected,
                    onCompose = onCompose,
                    onOpenThread = onOpenThread,
                    onCopy = onCopy,
                    canCompose = canCompose,
                    accounts = accounts,
                    foldersByAccount = foldersByAccount,
                    showSenderImages = showSenderImages,
                )
            }
        }
    }
}

@Composable
private fun ContactsSearchBar(
    query: String,
    onQueryChange: (String) -> Unit,
    sort: String,
    sortOpen: Boolean,
    onSortOpenChange: (Boolean) -> Unit,
    sortEnabled: Boolean,
    onSelectSort: (String) -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val focusRequester = remember { FocusRequester() }
    Row(
        Modifier
            .fillMaxWidth()
            .padding(start = 16.dp, end = 16.dp, bottom = 10.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Surface(
            modifier = Modifier.weight(1f).height(42.dp),
            shape = RoundedCornerShape(12.dp),
            color = colors.surfaceContainerHigh,
        ) {
            Row(
                Modifier
                    .fillMaxSize()
                    .padding(start = 12.dp, end = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    Icons.Filled.Search,
                    contentDescription = null,
                    tint = colors.onSurfaceVariant,
                    modifier = Modifier.size(18.dp),
                )
                BasicTextField(
                    value = query,
                    onValueChange = onQueryChange,
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    cursorBrush = SolidColor(colors.primary),
                    textStyle = MaterialTheme.typography.bodyMedium.copy(color = colors.onSurface),
                    modifier =
                        Modifier
                            .weight(1f)
                            .padding(start = 10.dp, end = 4.dp)
                            .focusRequester(focusRequester),
                    decorationBox = { innerTextField ->
                        Box(
                            Modifier.fillMaxWidth(),
                            contentAlignment = Alignment.CenterStart,
                        ) {
                            if (query.isEmpty()) {
                                Text(
                                    tr("contacts.search"),
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = colors.onSurfaceVariant,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            }
                            innerTextField()
                        }
                    },
                )
                if (query.isNotEmpty()) {
                    IconButton(
                        onClick = { onQueryChange("") },
                        modifier = Modifier.size(34.dp),
                    ) {
                        Icon(
                            Icons.Filled.Close,
                            contentDescription = tr("common.clearSearch"),
                            tint = colors.onSurfaceVariant,
                            modifier = Modifier.size(18.dp),
                        )
                    }
                }
            }
        }
        Box {
            Surface(
                modifier = Modifier.size(42.dp),
                shape = RoundedCornerShape(12.dp),
                color = if (sortOpen) colors.secondaryContainer else colors.surfaceContainerHigh,
            ) {
                IconButton(
                    onClick = { onSortOpenChange(true) },
                    enabled = sortEnabled,
                    modifier = Modifier.fillMaxSize(),
                ) {
                    Icon(
                        Icons.AutoMirrored.Filled.Sort,
                        contentDescription = tr("contacts.sort") + ": " + tr(contactSortLabel(sort)),
                        tint = if (sortOpen) colors.onSecondaryContainer else colors.onSurfaceVariant,
                        modifier = Modifier.size(20.dp),
                    )
                }
            }
            DropdownMenu(expanded = sortOpen, onDismissRequest = { onSortOpenChange(false) }) {
                listOf("frequent", "recent", "name").forEach { option ->
                    val activeSort = sort == option
                    DropdownMenuItem(
                        text = {
                            Text(
                                tr(contactSortLabel(option)),
                                fontWeight = if (activeSort) FontWeight.SemiBold else FontWeight.Normal,
                            )
                        },
                        onClick = { onSelectSort(option) },
                        trailingIcon = {
                            if (activeSort) {
                                Icon(Icons.Filled.Check, contentDescription = null, tint = colors.primary)
                            }
                        },
                    )
                }
            }
        }
    }
}

private fun contactSortLabel(sort: String): String =
    when (sort) {
        "recent" -> "contacts.sortRecent"
        "name" -> "contacts.sortName"
        else -> "contacts.sortFrequent"
    }

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun ContactDirectoryRow(
    state: ContactsScreenState,
    contact: ContactSuggestion,
    showSenderImages: Boolean,
    onSelect: () -> Unit,
    onPin: () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val pinned = state.isPinned(contact)
    val pinLabel = tr(if (pinned) "contacts.unpin" else "contacts.pin")
    val enabled = state.pinsLoaded && !state.pinsSaving
    var menuOpen by remember(contact.addr) { mutableStateOf(false) }
    var reveal by remember(contact.addr) { mutableFloatStateOf(0f) }
    val revealWidth = with(LocalDensity.current) { 88.dp.toPx() }
    val direction = if (LocalLayoutDirection.current == LayoutDirection.Rtl) -1f else 1f
    val displayName = contact.name.ifBlank { contact.addr }
    val hasDistinctName = contact.name.isNotBlank() && !contact.name.equals(contact.addr, ignoreCase = true)
    Box(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 8.dp, vertical = 1.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(if (reveal < -0.5f) colors.secondaryContainer else colors.surface),
    ) {
        if (reveal < -0.5f) {
            Box(
                modifier =
                    Modifier
                        .align(Alignment.CenterEnd)
                        .width(88.dp)
                        .heightIn(min = 58.dp)
                        .clickable(enabled = enabled) {
                            reveal = 0f
                            onPin()
                        }.padding(horizontal = 6.dp),
                contentAlignment = Alignment.Center,
            ) {
                Column(
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    Icon(
                        if (pinned) Icons.Outlined.PushPin else Icons.Filled.PushPin,
                        contentDescription = null,
                        tint = colors.onSecondaryContainer,
                        modifier = Modifier.size(18.dp),
                    )
                    Text(
                        pinLabel,
                        style = MaterialTheme.typography.labelSmall,
                        color = colors.onSecondaryContainer,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
        Row(
            Modifier
                .offset { IntOffset((reveal * direction).roundToInt(), 0) }
                .fillMaxWidth()
                .heightIn(min = 58.dp)
                .background(colors.surface)
                .semantics {
                    if (enabled) {
                        customActions =
                            listOf(
                                CustomAccessibilityAction(pinLabel) {
                                    reveal = 0f
                                    onPin()
                                    true
                                },
                            )
                    }
                }
                // A bounded reveal, never a dismiss: releasing exposes a button, not an action.
                .draggable(
                    state = rememberDraggableState { delta -> reveal = (reveal + delta * direction).coerceIn(-revealWidth, 0f) },
                    orientation = Orientation.Horizontal,
                    enabled = enabled,
                    onDragStopped = { velocity ->
                        val target = if (velocity * direction < -600f || (velocity * direction <= 600f && reveal < -revealWidth / 2)) -revealWidth else 0f
                        animate(reveal, target) { value, _ -> reveal = value }
                    },
                ).combinedClickable(
                    onClick = {
                        if (reveal < -0.5f) {
                            reveal = 0f
                        } else {
                            onSelect()
                        }
                    },
                    onLongClickLabel = pinLabel,
                    onLongClick = { menuOpen = true },
                ).padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SenderAvatar(
                formatContactSuggestion(contact),
                enabled = showSenderImages,
                size = 40.dp,
                fallbackName = displayName,
            )
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(
                    displayName,
                    style = MaterialTheme.typography.bodyMedium,
                    fontWeight = FontWeight.Medium,
                    color = colors.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (hasDistinctName) {
                    Text(
                        contact.addr,
                        style = MaterialTheme.typography.bodySmall,
                        color = colors.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            if (pinned) {
                Icon(
                    Icons.Filled.PushPin,
                    contentDescription = null,
                    tint = colors.primary.copy(alpha = 0.7f),
                    modifier = Modifier.size(15.dp),
                )
            }
        }
        DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
            ContactPinMenuItem(state, contact) {
                menuOpen = false
                reveal = 0f
                onPin()
            }
        }
    }
}

@Composable
private fun ContactPinMenuItem(
    state: ContactsScreenState,
    contact: ContactSuggestion,
    onClick: () -> Unit,
) {
    val pinned = state.isPinned(contact)
    DropdownMenuItem(
        text = { Text(tr(if (pinned) "contacts.unpin" else "contacts.pin")) },
        leadingIcon = { Icon(if (pinned) Icons.Outlined.PushPin else Icons.Filled.PushPin, null) },
        onClick = onClick,
        enabled = state.pinsLoaded && !state.pinsSaving,
    )
}

@Composable
internal fun ContactLoading() {
    Box(Modifier.fillMaxWidth().padding(20.dp), contentAlignment = Alignment.Center) {
        CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.5.dp)
    }
}

@Composable
internal fun ContactFailure(onRetry: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(colors.errorContainer.copy(alpha = 0.5f))
            .clickable(onClick = onRetry)
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(
            Icons.Filled.ErrorOutline,
            contentDescription = null,
            tint = colors.error,
            modifier = Modifier.size(18.dp),
        )
        Text(
            tr("contacts.retry"),
            style = MaterialTheme.typography.bodySmall,
            fontWeight = FontWeight.Medium,
            color = colors.error,
        )
    }
}
