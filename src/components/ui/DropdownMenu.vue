<template>
    <span v-if="$slots.trigger" ref="trigger" class="ui-dropdown__trigger">
        <slot name="trigger" :open="isOpen" :toggle="toggle" :attrs="triggerAttrs" />
    </span>
    <Teleport v-if="isOpen" :to="teleportTarget">
        <div
            :id="menuId"
            ref="menu"
            role="menu"
            :aria-label="label"
            class="ui-menu"
            :class="{ '-positioned': positioned }"
            :style="{ left: `${position.left}px`, top: `${position.top}px`, minWidth: minWidth ? `${minWidth}px` : undefined }"
            tabindex="-1"
            v-bind="$attrs"
            @keydown="onMenuKeydown"
            @click="onMenuClick"
        >
            <slot :close="close" />
        </div>
    </Teleport>
</template>

<script lang="ts">
    /**
     * The one menu of the app: dropdowns (⋮ buttons, sort pickers) and
     * context menus (right-click on a book).
     *
     * As a dropdown, the trigger goes in the `trigger` slot; bind the
     * provided `attrs` on it (aria-haspopup, aria-expanded, aria-controls,
     * and ArrowDown/ArrowUp to open from the keyboard) and call `toggle`:
     *
     *     <DropdownMenu :label="$t('detail.moreActions')" placement="bottom-end">
     *         <template #trigger="{ toggle, attrs }">
     *             <IconButton icon="more-vertical" :label="$t('detail.moreActions')" v-bind="attrs" @click="toggle" />
     *         </template>
     *         <MenuItem icon="folder" @click="reveal">{{ $t('detail.reveal') }}</MenuItem>
     *         <MenuSeparator />
     *         <MenuItem icon="trash-2" danger @click="remove">{{ $t('detail.delete') }}</MenuItem>
     *     </DropdownMenu>
     *
     * As a context menu, leave the trigger out and open it from the event:
     * `(this.$refs.menu as …).openFromEvent(event)` in a `@contextmenu`
     * handler.
     *
     * Behaviour (WAI-ARIA menu pattern):
     * - opening focuses the first item (the last one with ArrowUp);
     * - ArrowDown / ArrowUp move (and wrap), Home / End jump, a letter jumps
     *   to the next item starting with it; disabled items are skipped;
     * - choosing an item closes the menu (unless the item has
     *   `data-keep-open`) and puts focus back on the trigger;
     * - Escape and Tab close it and put focus back; a click outside closes
     *   it without moving focus; so do scrolling and resizing the window;
     * - it is positioned (fixed) next to the trigger or at the pointer,
     *   flipped and clamped so it stays inside the window.
     *
     * It is rendered at the end of <body>, or inside the open dialog that
     * contains the trigger: content outside a modal dialog is inert, and
     * the dialog is in the top layer, above anything else.
     */
    import { defineComponent, nextTick, type PropType } from 'vue'
    import { computeMenuPosition, type Box } from './menuPosition'
    import { uniqueId } from '@/utils/dom'
    import type { MenuPlacement } from './types'

    const ITEM_SELECTOR = '[role="menuitem"], [role="menuitemcheckbox"], [role="menuitemradio"]'

    export default defineComponent({
        name: 'DropdownMenu',

        // Attributes (class, data-*) go on the menu itself, not the trigger.
        inheritAttrs: false,

        props: {
            /** Accessible name of the menu ("More actions", "Sort by"). */
            label:     { type: String, required: true },
            placement: { type: String as PropType<MenuPlacement>, default: 'bottom-start' },
            /** Gap between the trigger and the menu, in px. */
            offset:    { type: Number, default: 4 },
            /** Minimum width in px (0: as wide as the items). */
            minWidth:  { type: Number, default: 0 },
        },

        emits: ['open', 'close'],

        data(): {
            isOpen: boolean
            positioned: boolean
            position: { left: number, top: number }
            /** Where the menu opens: the trigger's box, or a point (zero-size box). */
            anchorBox: Box | null
            teleportTarget: string | HTMLElement
            menuId: string
            /** Where focus goes back when the menu closes. */
            returnFocusTo: HTMLElement | null
            typeahead: string
            typeaheadTimer: ReturnType<typeof setTimeout> | null
        } {
            return {
                isOpen: false,
                positioned: false,
                position: { left: 0, top: 0 },
                anchorBox: null,
                teleportTarget: 'body',
                menuId: uniqueId('menu'),
                returnFocusTo: null,
                typeahead: '',
                typeaheadTimer: null,
            }
        },

        computed: {
            /** To bind on the trigger element (see the slot example above). */
            triggerAttrs(): Record<string, unknown> {
                return {
                    'aria-haspopup': 'menu',
                    'aria-expanded': String(this.isOpen),
                    'aria-controls': this.isOpen ? this.menuId : undefined,
                    onKeydown: this.onTriggerKeydown,
                }
            },
        },

        beforeUnmount() {
            this.removeGlobalListeners()
            if (this.typeaheadTimer) { clearTimeout(this.typeaheadTimer) }
        },

        methods: {
            triggerElement(): HTMLElement | null {
                const wrapper = this.$refs.trigger as HTMLElement | undefined
                return (wrapper?.firstElementChild as HTMLElement | null) ?? wrapper ?? null
            },

            /** Opens below/above the trigger. */
            open(focus: 'first' | 'last' = 'first') {
                const trigger = this.triggerElement()
                if (!trigger) { return }
                this.show(boxOf(trigger), trigger, trigger, focus)
            },

            /** Opens with its corner at (x, y), viewport coordinates. */
            openAt(x: number, y: number, origin: Element | null = null) {
                const returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null
                this.show({ left: x, top: y, width: 0, height: 0 }, origin, returnTo, 'first')
            },

            /**
             * Opens as a context menu for a `contextmenu` (or click) event.
             * From the keyboard (context-menu key, Shift+F10) there is no
             * pointer position: the menu opens under the event's target.
             */
            openFromEvent(e: MouseEvent) {
                e.preventDefault()
                const target = e.target instanceof Element ? e.target : null
                if (e.clientX === 0 && e.clientY === 0 && target) {
                    const box = boxOf(target)
                    this.openAt(box.left, box.top + box.height, target)
                } else {
                    this.openAt(e.clientX, e.clientY, target)
                }
            },

            toggle() {
                if (this.isOpen) { this.close() } else { this.open() }
            },

            show(anchor: Box, origin: Element | null, returnTo: HTMLElement | null, focus: 'first' | 'last') {
                if (this.isOpen) { this.close({ restoreFocus: false }) }
                this.anchorBox = anchor
                this.returnFocusTo = returnTo
                // Inside an open modal dialog, the menu must live in it (see above).
                this.teleportTarget = (origin?.closest('dialog[open]') as HTMLElement | null) ?? 'body'
                this.positioned = false
                this.isOpen = true
                this.addGlobalListeners()
                this.$emit('open')
                // Render, measure and place it, then focus an item once it is
                // visible (a visibility: hidden element can't take focus).
                void nextTick(() => {
                    this.reposition()
                    void nextTick(() => this.focusItem(focus === 'first' ? 0 : -1))
                })
            },

            /** Closes the menu; focus goes back to the trigger unless told otherwise. */
            close(options: { restoreFocus?: boolean } = {}) {
                if (!this.isOpen) { return }
                this.isOpen = false
                this.removeGlobalListeners()
                this.$emit('close')
                if (options.restoreFocus !== false && this.returnFocusTo?.isConnected) {
                    this.returnFocusTo.focus()
                }
            },

            reposition() {
                const menu = this.$refs.menu as HTMLElement | undefined
                if (!menu || !this.anchorBox) { return }
                const size = menu.getBoundingClientRect()
                this.position = computeMenuPosition(
                    this.anchorBox,
                    { width: size.width, height: size.height },
                    { width: window.innerWidth, height: window.innerHeight },
                    this.placement,
                    this.offset,
                )
                this.positioned = true
            },

            items(): HTMLElement[] {
                const menu = this.$refs.menu as HTMLElement | undefined
                if (!menu) { return [] }
                return Array.from(menu.querySelectorAll<HTMLElement>(ITEM_SELECTOR))
                    .filter(el => el.getAttribute('aria-disabled') !== 'true' && !el.hasAttribute('disabled'))
            },

            /** Focuses item `index` (negative counts from the end); the menu itself when empty. */
            focusItem(index: number) {
                const items = this.items()
                if (items.length === 0) {
                    (this.$refs.menu as HTMLElement | undefined)?.focus()
                    return
                }
                const i = ((index % items.length) + items.length) % items.length
                items[i].focus()
            },

            moveFocus(delta: number) {
                const items = this.items()
                const current = items.indexOf(document.activeElement as HTMLElement)
                this.focusItem(current === -1 ? (delta > 0 ? 0 : -1) : current + delta)
            },

            onTriggerKeydown(e: KeyboardEvent) {
                if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
                    e.preventDefault()
                    this.open(e.key === 'ArrowDown' ? 'first' : 'last')
                }
            },

            onMenuKeydown(e: KeyboardEvent) {
                // Keys used here must not also reach a dialog or a view
                // (Escape would otherwise close the dialog around the menu).
                const handled = () => { e.preventDefault(); e.stopPropagation() }
                switch (e.key) {
                    case 'ArrowDown': handled(); this.moveFocus(1); return
                    case 'ArrowUp':   handled(); this.moveFocus(-1); return
                    case 'Home':      handled(); this.focusItem(0); return
                    case 'End':       handled(); this.focusItem(-1); return
                    case 'Escape':    handled(); this.close(); return
                    case 'Tab':       handled(); this.close(); return
                    default:
                        if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && e.key !== ' ') {
                            handled()
                            this.jumpTo(e.key)
                        }
                }
            },

            /** Type-ahead: the next item whose text starts with what was typed. */
            jumpTo(char: string) {
                if (this.typeaheadTimer) { clearTimeout(this.typeaheadTimer) }
                this.typeahead += char.toLowerCase()
                this.typeaheadTimer = setTimeout(() => { this.typeahead = '' }, 500)
                const items = this.items()
                if (items.length === 0) { return }
                // A new letter searches from the item after the current one
                // (pressing "d" twice visits each "D…" item); a longer prefix
                // being typed keeps matching from the current one.
                const current = items.indexOf(document.activeElement as HTMLElement)
                const from = current === -1 ? 0 : current + (this.typeahead.length === 1 ? 1 : 0)
                const ordered = items.map((_, k) => items[(from + k) % items.length])
                const match = ordered.find(el => (el.textContent ?? '').trim().toLowerCase().startsWith(this.typeahead))
                match?.focus()
            },

            onMenuClick(e: MouseEvent) {
                const item = e.target instanceof Element ? e.target.closest<HTMLElement>(ITEM_SELECTOR) : null
                if (!item || item.getAttribute('aria-disabled') === 'true' || item.hasAttribute('data-keep-open')) { return }
                this.close()
            },

            onDocumentPointerdown(e: Event) {
                const target = e.target as Node | null
                const menu = this.$refs.menu as HTMLElement | undefined
                const trigger = this.$refs.trigger as HTMLElement | undefined
                if (target && (menu?.contains(target) || trigger?.contains(target))) { return }
                this.close({ restoreFocus: false })
            },

            onScroll(e: Event) {
                const menu = this.$refs.menu as HTMLElement | undefined
                if (menu && e.target instanceof Node && menu.contains(e.target)) { return }
                this.close({ restoreFocus: false })
            },

            onResize() {
                this.close({ restoreFocus: false })
            },

            addGlobalListeners() {
                document.addEventListener('pointerdown', this.onDocumentPointerdown, true)
                document.addEventListener('scroll', this.onScroll, true)
                window.addEventListener('resize', this.onResize)
                window.addEventListener('blur', this.onResize)
            },

            removeGlobalListeners() {
                document.removeEventListener('pointerdown', this.onDocumentPointerdown, true)
                document.removeEventListener('scroll', this.onScroll, true)
                window.removeEventListener('resize', this.onResize)
                window.removeEventListener('blur', this.onResize)
            },
        },
    })

    function boxOf(el: Element): Box {
        const r = el.getBoundingClientRect()
        return { left: r.left, top: r.top, width: r.width, height: r.height }
    }
</script>

<style lang="scss" scoped>
.ui-dropdown__trigger { display: inline-flex; }

.ui-menu {
    position: fixed;
    z-index: var(--z-popover);
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 180px;
    max-width: min(320px, calc(100vw - 16px));
    max-height: calc(100vh - 16px);
    overflow-y: auto;
    padding: var(--space-1);
    background: color-mix(in srgb, var(--bg-elevated) 92%, transparent);
    backdrop-filter: blur(16px) saturate(130%);
    -webkit-backdrop-filter: blur(16px) saturate(130%);
    border: 1px solid var(--hairline);
    border-radius: var(--radius);
    box-shadow: var(--shadow-popover), inset 0 1px 0 rgba(255,255,255,.05);
    outline: none;
    // Measured before it is placed: invisible until then (no flash at 0,0).
    visibility: hidden;

    &.-positioned {
        visibility: visible;
        animation: ui-menu-in var(--duration-fast) var(--ease-out);
    }
}

@keyframes ui-menu-in {
    from { opacity: 0; transform: translateY(-2px); }
    to   { opacity: 1; transform: none; }
}
</style>
