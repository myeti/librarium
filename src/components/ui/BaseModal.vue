<template>
    <Teleport to="body">
        <!--
            The <dialog> element is the full-screen backdrop and the panel is
            the visible box: a click on the dialog itself is a click outside
            the panel. showModal() puts it in the browser's top layer, makes
            the rest of the page inert and gives Escape to the top dialog.
        -->
        <dialog
            ref="dialog"
            class="ui-modal"
            :class="{ '-screen': size === 'screen' }"
            :role="role"
            aria-modal="true"
            :aria-labelledby="labelledbyId"
            :aria-label="labelledbyId ? undefined : (ariaLabel || undefined)"
            :aria-describedby="describedby || undefined"
            @keydown="onKeydown"
            @cancel="onCancel"
            @close="onNativeClose"
            @mousedown="onBackdropMousedown"
            @click="onBackdropClick"
        >
            <div
                ref="panel"
                class="ui-modal__panel"
                :class="[`-${size}`, { '-bare': bare }]"
                :style="width ? { width: `${width}px` } : undefined"
                tabindex="-1"
                v-bind="$attrs"
            >
                <template v-if="!bare">
                    <header v-if="title || $slots.title || $slots.header || showCloseButton" class="ui-modal__header">
                        <slot name="header">
                            <h2 :id="titleId" class="ui-modal__title">
                                <slot name="title">{{ title }}</slot>
                            </h2>
                        </slot>
                        <IconButton
                            v-if="showCloseButton"
                            icon="x"
                            :label="$t('common.close')"
                            class="ui-modal__close"
                            @click="requestClose('button')"
                        />
                    </header>
                    <div class="ui-modal__body">
                        <slot />
                    </div>
                    <footer v-if="$slots.footer" class="ui-modal__footer">
                        <slot name="footer" />
                    </footer>
                </template>
                <slot v-else />
            </div>
        </dialog>

        <!-- "Discard your changes?" — shown instead of closing while `dirty`. -->
        <BaseModal
            v-if="guardOpen"
            bare
            size="sm"
            role="alertdialog"
            :labelledby="guardTitleId"
            :describedby="guardBodyId"
            :show-close="false"
            @close="guardOpen = false"
        >
            <div class="ui-modal__guard">
                <h2 :id="guardTitleId" class="ui-modal__guard-title">{{ $t('ui.discardTitle') }}</h2>
                <p :id="guardBodyId" class="ui-modal__guard-text">{{ $t('ui.discardMessage') }}</p>
                <div class="ui-modal__guard-actions">
                    <BaseButton variant="ghost" data-autofocus @click="guardOpen = false">{{ $t('ui.keepEditing') }}</BaseButton>
                    <BaseButton variant="danger" @click="discard">{{ $t('ui.discard') }}</BaseButton>
                </div>
            </div>
        </BaseModal>
    </Teleport>
</template>

<script lang="ts">
    /**
     * The dialog every modal of the app is built on: a native <dialog>
     * opened with showModal().
     *
     * Mount it with `v-if` (it opens when mounted, closes when unmounted) and
     * handle `close`, which is a *request*: the parent decides, usually by
     * unmounting it.
     *
     *     <BaseModal v-if="editing" :title="$t('metadata.title')" :dirty="changed" @close="editing = false">
     *         …form…
     *         <template #footer>
     *             <BaseButton variant="ghost" @click="editing = false">{{ $t('common.cancel') }}</BaseButton>
     *             <BaseButton variant="primary" @click="save">{{ $t('common.save') }}</BaseButton>
     *         </template>
     *     </BaseModal>
     *
     * What it takes care of:
     * - **Top layer and inert page** (the browser, through showModal()):
     *   nothing behind the dialog can be clicked or focused.
     * - **Stack**: it registers in the modals store; only the dialog on top
     *   reacts to keys, and global shortcuts are off while one is open. Key
     *   events don't leave the dialog (no view shortcut sees them).
     * - **Escape** asks to close the top dialog only (`close` with reason
     *   'escape'); ignored while `dismissible` is false (an action running).
     * - **Focus**: moves in on open (an element marked `data-autofocus` or
     *   `autofocus`, else the first focusable element of the body, else the
     *   panel), stays in with Tab / Shift+Tab, and goes back to where it was
     *   on close.
     * - **Labels**: `aria-labelledby` points at the title (or at `labelledby`
     *   when the content renders its own heading); `describedby` is optional.
     * - **Backdrop**: a click outside the panel closes only with
     *   `closeOnBackdrop` (off by default: a stray click must not lose work).
     * - **Unsaved edits**: with `dirty`, Escape, the × and the backdrop ask
     *   "Discard your changes?" first; nothing is discarded without a yes.
     *   Closing from your own buttons (Save, Cancel) is up to you.
     *
     * Layout: a header (title + ×), a scrolling body and an optional footer
     * for the actions; or `bare`, where the content lays itself out (alert
     * dialogs such as ConfirmDialog). Sizes: sm 440px, md 560px, lg 760px,
     * xl 960px, full (the window minus a margin), screen (the whole window,
     * no margin, corners or backdrop: the reader); `width` overrides.
     *
     * Attributes (class, style, data-*) go on the panel.
     */
    import { defineComponent, type PropType } from 'vue'
    import { mapStores } from 'pinia'
    import { useModalsStore } from '@/stores/modals'
    import { focusableWithin, uniqueId } from '@/utils/dom'
    import BaseButton from './BaseButton.vue'
    import IconButton from './IconButton.vue'
    import type { ModalCloseReason, ModalSize } from './types'

    /** A cancel event this close (ms) after an Escape keydown is the same key press. */
    const ESCAPE_DEDUP_MS = 100

    export default defineComponent({
        name: 'BaseModal',

        components: { BaseButton, IconButton },

        inheritAttrs: false,

        props: {
            /** Heading text; it labels the dialog. Or use the `title` / `header` slot. */
            title:        { type: String, default: '' },
            /** Id of a heading rendered by the content (bare dialogs). */
            labelledby:   { type: String, default: '' },
            /** Accessible name when there is no visible heading at all. */
            ariaLabel:    { type: String, default: '' },
            /** Id of the element that describes the dialog (its message). */
            describedby:  { type: String, default: '' },
            size:         { type: String as PropType<ModalSize>, default: 'md' },
            /** Panel width in px, overriding `size`. */
            width:        { type: Number, default: 0 },
            role:         { type: String as PropType<'dialog' | 'alertdialog'>, default: 'dialog' },
            /** Escape, × and backdrop may ask to close. False while an action runs. */
            dismissible:  { type: Boolean, default: true },
            /** A click on the backdrop asks to close. */
            closeOnBackdrop: { type: Boolean, default: false },
            /** Unsaved edits: closing by Escape, × or backdrop asks for confirmation first. */
            dirty:        { type: Boolean, default: false },
            /** Show the × button in the header. */
            showClose:    { type: Boolean, default: true },
            /** No header/body/footer chrome: the content lays itself out. */
            bare:         { type: Boolean, default: false },
            /** CSS selector (inside the dialog) of the element to focus first. */
            initialFocus: { type: String, default: '' },
        },

        emits: {
            /** The user asked to close (Escape, ×, backdrop). The parent closes it. */
            close: (_reason: ModalCloseReason) => true,
            /** Keys not handled by the dialog itself (Escape and Tab are). */
            keydown: (_e: KeyboardEvent) => true,
        },

        data(): {
            modalId: string
            titleId: string
            guardTitleId: string
            guardBodyId: string
            guardOpen: boolean
            guardReason: ModalCloseReason
            previouslyFocused: HTMLElement | null
            /** Set while we close the <dialog> ourselves (its `close` event is then expected). */
            closingOurselves: boolean
            mousedownOnBackdrop: boolean
            lastEscapeAt: number
        } {
            return {
                modalId: '',
                titleId: uniqueId('modal-title'),
                guardTitleId: uniqueId('modal-guard-title'),
                guardBodyId: uniqueId('modal-guard-body'),
                guardOpen: false,
                guardReason: 'escape',
                previouslyFocused: null,
                closingOurselves: false,
                mousedownOnBackdrop: false,
                lastEscapeAt: -Infinity,
            }
        },

        computed: {
            ...mapStores(useModalsStore),

            labelledbyId(): string | undefined {
                if (this.labelledby) { return this.labelledby }
                return (this.title || this.$slots.title) && !this.bare ? this.titleId : undefined
            },

            showCloseButton(): boolean {
                return this.showClose && this.dismissible
            },

            /** Only the top dialog reacts to keys. */
            isTop(): boolean {
                return this.modalsStore.isTop(this.modalId)
            },
        },

        mounted() {
            this.previouslyFocused = document.activeElement instanceof HTMLElement ? document.activeElement : null
            const owner = this.$parent?.$options.name ?? 'BaseModal'
            this.modalId = this.modalsStore.open(owner)

            const dialog = this.$refs.dialog as HTMLDialogElement
            if (!dialog.open) {
                try {
                    dialog.showModal()
                } catch {
                    // Not supported or not connected (unusual): show it anyway.
                    dialog.setAttribute('open', '')
                }
            }
            this.focusInitial()
        },

        beforeUnmount() {
            this.modalsStore.close(this.modalId)
            const dialog = this.$refs.dialog as HTMLDialogElement | undefined
            if (dialog?.open) {
                this.closingOurselves = true
                dialog.close()
            }
        },

        unmounted() {
            // Give focus back, unless the parent already moved it somewhere
            // meaningful, or the opener is gone (a deleted book's button).
            const active = document.activeElement
            const focusIsLost = !active || active === document.body
            if (focusIsLost && this.previouslyFocused?.isConnected) {
                this.previouslyFocused.focus()
            }
        },

        methods: {
            /** Asks the parent to close, through the discard guard when `dirty`. */
            requestClose(reason: ModalCloseReason) {
                if (!this.dismissible || this.guardOpen) { return }
                if (this.dirty) {
                    this.guardReason = reason
                    this.guardOpen = true
                    return
                }
                this.$emit('close', reason)
            },

            discard() {
                this.guardOpen = false
                this.$emit('close', this.guardReason)
            },

            focusInitial() {
                const panel = this.$refs.panel as HTMLElement
                const body = panel.querySelector<HTMLElement>('.ui-modal__body') ?? panel
                const target = (this.initialFocus ? panel.querySelector<HTMLElement>(this.initialFocus) : null)
                    ?? panel.querySelector<HTMLElement>('[data-autofocus], [autofocus]')
                    ?? focusableWithin(body)[0]
                    ?? panel
                target.focus()
            },

            onKeydown(e: KeyboardEvent) {
                // The dialog owns the keyboard: nothing behind it reacts.
                e.stopPropagation()
                if (!this.isTop) { return }
                if (e.key === 'Escape') {
                    // preventDefault: the browser must not also close the
                    // <dialog> on its own (we decide, maybe through the guard).
                    e.preventDefault()
                    this.lastEscapeAt = performance.now()
                    this.requestClose('escape')
                    return
                }
                if (e.key === 'Tab') {
                    this.trapFocus(e)
                    return
                }
                this.$emit('keydown', e)
            },

            /** The browser's own close request (Escape, or a platform gesture). */
            onCancel(e: Event) {
                e.preventDefault()
                if (!this.isTop) { return }
                if (performance.now() - this.lastEscapeAt < ESCAPE_DEDUP_MS) { return }
                this.requestClose('escape')
            },

            /**
             * The <dialog> closed. Expected when we close it on unmount. If the
             * browser closed it on its own (Chromium does after repeated
             * Escapes), show it again and treat it as a close request: the
             * parent's state, not the browser, decides whether it is open.
             */
            onNativeClose() {
                if (this.closingOurselves) { return }
                const dialog = this.$refs.dialog as HTMLDialogElement | undefined
                if (!dialog || !dialog.isConnected) { return }
                try { dialog.showModal() } catch { dialog.setAttribute('open', '') }
                this.requestClose('escape')
            },

            onBackdropMousedown(e: MouseEvent) {
                // Only a press that starts on the backdrop counts: selecting
                // text in a field and releasing outside must not close.
                this.mousedownOnBackdrop = e.target === this.$refs.dialog
            },

            onBackdropClick(e: MouseEvent) {
                const onBackdrop = e.target === this.$refs.dialog && this.mousedownOnBackdrop
                this.mousedownOnBackdrop = false
                if (onBackdrop && this.closeOnBackdrop) { this.requestClose('backdrop') }
            },

            /**
             * Keeps Tab inside the dialog. Browsers already confine focus to a
             * modal dialog, but some let it escape to the window chrome; this
             * makes it wrap around, the same everywhere.
             */
            trapFocus(e: KeyboardEvent) {
                const panel = this.$refs.panel as HTMLElement
                const items = focusableWithin(panel)
                if (items.length === 0) {
                    e.preventDefault()
                    panel.focus()
                    return
                }
                const first = items[0]
                const last = items[items.length - 1]
                const active = document.activeElement
                if (e.shiftKey && (active === first || active === panel || !panel.contains(active))) {
                    e.preventDefault()
                    last.focus()
                } else if (!e.shiftKey && (active === last || !panel.contains(active))) {
                    e.preventDefault()
                    first.focus()
                }
            },
        },
    })
</script>

<style lang="scss" scoped>
// The <dialog> is the backdrop: it covers the window.
.ui-modal {
    position: fixed;
    inset: 0;
    width: 100%;
    height: 100%;
    max-width: none;
    max-height: none;
    margin: 0;
    padding: var(--space-5);
    border: none;
    background: var(--modal-backdrop);
    backdrop-filter: blur(10px) saturate(120%);
    -webkit-backdrop-filter: blur(10px) saturate(120%);
    color: var(--text-primary);
    overflow: hidden;

    // Only when open: a closed <dialog> must stay display: none.
    &[open] {
        display: flex;
        align-items: center;
        justify-content: center;
    }

    // `screen`: edge to edge, nothing of the page shows around it.
    &.-screen {
        padding: 0;
        background: var(--bg);
        backdrop-filter: none;
        -webkit-backdrop-filter: none;
    }

    // The real ::backdrop stays transparent: the dialog draws its own (above),
    // because older WebKit doesn't give ::backdrop the theme's variables.
    &::backdrop { background: transparent; }
}

.ui-modal__panel {
    position: relative;
    display: flex;
    flex-direction: column;
    max-width: 100%;
    max-height: 100%;
    background: var(--bg-modal);
    border: 1px solid var(--hairline);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-lg), inset 0 1px 0 rgba(255,255,255,.06);
    overflow: hidden;
    outline: none;
    animation: ui-modal-in var(--duration-slow) var(--ease-out);

    // The theme's corner ornaments, top left and top right, in its metal.
    &:not(.-screen):not(.-bare)::before,
    &:not(.-screen):not(.-bare)::after {
        content: "";
        position: absolute;
        top: 7px;
        width: 34px;
        aspect-ratio: 1;
        background: var(--foil);
        -webkit-mask: var(--ornament-corner) center / contain no-repeat;
        mask: var(--ornament-corner) center / contain no-repeat;
        opacity: .55;
        pointer-events: none;
    }
    &::before { left: 7px; }
    &::after { right: 7px; transform: scaleX(-1); }

    &.-sm { width: 440px; border-radius: var(--radius); }
    &.-md { width: 560px; }
    &.-lg { width: 760px; }
    &.-xl { width: 960px; }
    &.-full { width: 100%; height: 100%; }
    &.-screen { width: 100%; height: 100%; border: none; border-radius: 0; box-shadow: none; }

    &.-bare { overflow-y: auto; }
}

@keyframes ui-modal-in {
    from { opacity: 0; transform: translateY(10px) scale(0.975); filter: blur(2px); }
    to   { opacity: 1; transform: none; filter: none; }
}

// Header and footer are set off by a gilt line that fades at both ends.
.ui-modal__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    flex-shrink: 0;
    padding: 22px 28px 18px;
    background: linear-gradient(90deg, transparent, var(--hairline) 20%, var(--hairline) 80%, transparent) bottom / 100% 1px no-repeat;
}

.ui-modal__title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    font-family: var(--font-heading);
    font-size: 19px;
    font-weight: var(--heading-weight);
    letter-spacing: var(--heading-tracking);
    line-height: var(--line-height-tight);
    color: var(--text-primary);
}

.ui-modal__close { margin: -6px -8px -6px 0; }

.ui-modal__body {
    flex: 1 1 auto;
    min-height: 0;
    padding: 24px 28px;
    overflow-y: auto;
}

.ui-modal__footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
    flex-shrink: 0;
    padding: 16px 28px 18px;
    background: linear-gradient(90deg, transparent, var(--hairline) 20%, var(--hairline) 80%, transparent) top / 100% 1px no-repeat;
}

// The discard guard (a bare alert dialog).
.ui-modal__guard { padding: 20px 22px 16px; }
.ui-modal__guard-title {
    font-family: var(--font-heading);
    font-size: var(--font-size-lg);
    font-weight: var(--heading-weight);
    margin-bottom: var(--space-2);
}
.ui-modal__guard-text {
    font-size: var(--font-size-sm);
    color: var(--text-secondary);
}
.ui-modal__guard-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: 18px;
}
</style>
