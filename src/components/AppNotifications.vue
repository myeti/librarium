<template>
    <Teleport :to="target">
        <!--
            A manual popover: shown once and kept open, it sits in the
            browser's top layer, above everything, modal dialogs included
            (see the script). Empty, it has no size and lets clicks through.
        -->
        <div
            ref="region"
            class="toasts"
            popover="manual"
            @mouseenter="hovered = true"
            @mouseleave="hovered = false"
            @focusin="focused = true"
            @focusout="onFocusOut"
        >
            <!-- Two live regions: confirmations wait their turn, errors interrupt. -->
            <TransitionGroup tag="ol" name="toast" class="toasts__list" aria-live="polite" aria-relevant="additions text">
                <li
                    v-for="n in politeItems"
                    :key="n.id"
                    class="toast"
                    :class="`-${n.type}`"
                    :data-type="n.type"
                    @keydown.esc.stop.prevent="dismiss(n.id)"
                >
                    <vue-feather :type="iconFor(n.type)" size="16" class="toast__icon" aria-hidden="true" />
                    <p class="toast__message"><span class="sr-only">{{ typeLabel(n.type) }}</span>{{ n.message }}</p>
                    <BaseButton v-if="n.action" size="sm" variant="ghost" class="toast__action" @click="runAction(n.id)">
                        {{ n.action.label }}
                    </BaseButton>
                    <IconButton icon="x" size="sm" :label="$t('notifications.dismiss')" class="toast__close" @click="dismiss(n.id)" />
                </li>
            </TransitionGroup>
            <TransitionGroup tag="ol" name="toast" class="toasts__list" aria-live="assertive" aria-relevant="additions text">
                <li
                    v-for="n in errorItems"
                    :key="n.id"
                    class="toast -error"
                    data-type="error"
                    @keydown.esc.stop.prevent="dismiss(n.id)"
                >
                    <vue-feather type="alert-circle" size="16" class="toast__icon" aria-hidden="true" />
                    <p class="toast__message"><span class="sr-only">{{ typeLabel('error') }}</span>{{ n.message }}</p>
                    <BaseButton v-if="n.action" size="sm" variant="ghost" class="toast__action" @click="runAction(n.id)">
                        {{ n.action.label }}
                    </BaseButton>
                    <IconButton icon="x" size="sm" :label="$t('notifications.dismiss')" class="toast__close" @click="dismiss(n.id)" />
                </li>
            </TransitionGroup>
        </div>
    </Teleport>
</template>

<script lang="ts">
    /**
     * The toasts of the notifications store, at the bottom of the window.
     *
     * **Above dialogs.** BaseModal dialogs are in the browser's top layer,
     * above any z-index, and a modal dialog makes everything outside it
     * inert (not clickable, not read by screen readers). So the toasts are:
     * - a `popover="manual"` element, shown with showPopover(): it joins the
     *   top layer *after* the dialog, so it is drawn above it;
     * - moved (Teleport) into the top modal dialog while one is open, so they
     *   are not inert: the × and the action buttons work, and errors are
     *   announced. When the dialog closes they move back to <body>.
     * Moving an element out of the document hides its popover, so it is
     * shown again after every move. Without the Popover API (old WebKit) the
     * same element is a plain fixed box (z-index), still inside the dialog.
     *
     * **Screen readers.** Two live regions that stay in place: info, success
     * and warnings are polite (read when the user is idle), errors are
     * assertive. Each toast starts with its kind ("Error:") for readers;
     * on screen the icon says it.
     *
     * **Time.** Timers stop while the pointer or the keyboard focus is on
     * the toasts, and while the window is hidden. Escape on a focused toast
     * dismisses it (and doesn't close the dialog behind it).
     */
    import { defineComponent } from 'vue'
    import { mapActions, mapState } from 'pinia'
    import { useNotificationsStore, type AppNotification, type NotificationType } from '@/stores/notifications'
    import { useModalsStore } from '@/stores/modals'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import IconButton from '@/components/ui/IconButton.vue'
    import type { TextKey } from '@/i18n'

    /** What a screen reader hears before the message (on screen, the icon says it). */
    const TYPE_LABELS: Record<NotificationType, TextKey> = {
        error:   'notifications.types.error',
        warning: 'notifications.types.warning',
        success: 'notifications.types.success',
        info:    'notifications.types.info',
    }

    /** The top open modal dialog (BaseModal), whose content is the only non-inert part of the page. */
    function topModalDialog(): HTMLDialogElement | null {
        const open = document.querySelectorAll<HTMLDialogElement>('dialog[open][aria-modal="true"]')
        return open.length ? open[open.length - 1] : null
    }

    function supportsPopover(el: HTMLElement): boolean {
        return typeof el.showPopover === 'function'
    }

    export default defineComponent({
        name: 'AppNotifications',

        components: { BaseButton, IconButton },

        data(): { target: string | HTMLElement, hovered: boolean, focused: boolean, windowHidden: boolean } {
            return {
                target: 'body',
                hovered: false,
                focused: false,
                windowHidden: false,
            }
        },

        computed: {
            ...mapState(useNotificationsStore, ['items']),
            ...mapState(useModalsStore, ['topId']),

            politeItems(): AppNotification[] {
                return this.items.filter(n => n.type !== 'error')
            },

            errorItems(): AppNotification[] {
                return this.items.filter(n => n.type === 'error')
            },

            shouldPause(): boolean {
                return this.hovered || this.focused || this.windowHidden
            },
        },

        watch: {
            // A dialog opened or closed: follow the top one (after the DOM
            // update, so the new dialog is there and the old one is gone).
            topId: {
                handler() { this.retarget() },
                flush: 'post',
            },

            shouldPause(pause: boolean) {
                if (pause) { this.pause() } else { this.resume() }
            },
        },

        mounted() {
            document.addEventListener('visibilitychange', this.onVisibilityChange)
            this.retarget()
            this.raise()
        },

        // Runs after every re-render, including a move to another dialog.
        updated() {
            this.raise()
        },

        beforeUnmount() {
            document.removeEventListener('visibilitychange', this.onVisibilityChange)
            if (this.shouldPause) { this.resume() }
        },

        methods: {
            ...mapActions(useNotificationsStore, ['dismiss', 'runAction', 'pause', 'resume']),

            retarget() {
                const next = topModalDialog() ?? 'body'
                if (next === this.target) { return }
                // The pointer and focus can't stay on an element being moved.
                this.hovered = false
                this.focused = false
                this.target = next
            },

            /** Shows the popover (top layer) if it isn't: on mount and after each move. */
            raise() {
                const region = this.$refs.region as HTMLElement | undefined
                if (!region || !region.isConnected || !supportsPopover(region)) { return }
                if (region.matches(':popover-open')) { return }
                try {
                    region.showPopover()
                } catch (e) {
                    // Shown as a plain fixed box instead (see the styles).
                    console.warn('[notifications] could not show the toasts in the top layer', e)
                }
            },

            onFocusOut(e: FocusEvent) {
                const region = this.$refs.region as HTMLElement | undefined
                if (!region?.contains(e.relatedTarget as Node | null)) { this.focused = false }
            },

            onVisibilityChange() {
                this.windowHidden = document.visibilityState === 'hidden'
            },

            iconFor(type: NotificationType): string {
                switch (type) {
                    case 'error':   return 'alert-circle'
                    case 'warning': return 'alert-triangle'
                    case 'success': return 'check-circle'
                    default:        return 'info'
                }
            },

            /** "Error: ", read before the message (the icon is hidden from screen readers). */
            typeLabel(type: NotificationType): string {
                return `${this.$t(TYPE_LABELS[type])} `
            },
        },
    })
</script>

<style lang="scss" scoped>
// The popover's own UA styles (centered box, border, Canvas colours) are
// reset here: it is a transparent, click-through stack at the bottom.
.toasts {
    position: fixed;
    inset: auto auto var(--space-5) 50%;
    transform: translateX(-50%);
    z-index: var(--z-toast);   // without the Popover API (top layer)
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    width: max-content;
    max-width: min(460px, calc(100vw - 2 * var(--space-4)));
    margin: 0;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--text-primary);
    overflow: visible;
    pointer-events: none;
}

.toasts__list {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    margin: 0;
    padding: 0;
    list-style: none;

    &:empty { display: none; }
}

.toast {
    --tone: var(--accent-text);
    --tone-tint: var(--accent-muted);

    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 280px;
    max-width: 100%;
    padding: 6px 6px 6px 14px;
    border: 1px solid color-mix(in srgb, var(--tone) 40%, transparent);
    border-radius: var(--radius);
    // The tint over an opaque surface: readable over a dialog or the grid.
    background: linear-gradient(var(--tone-tint), var(--tone-tint)), var(--bg-elevated);
    box-shadow: var(--shadow-lg);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    pointer-events: auto;

    &.-error   { --tone: var(--danger);  --tone-tint: var(--danger-muted); }
    &.-warning { --tone: var(--warning); --tone-tint: var(--warning-muted); }
    &.-success { --tone: var(--success); --tone-tint: var(--success-muted); }
}

.toast__icon {
    flex-shrink: 0;
    color: var(--tone);
}

.toast__message {
    flex: 1;
    min-width: 0;
    padding: 4px 0;
    line-height: var(--line-height-tight);
    word-break: break-word;
}

.toast__action {
    flex-shrink: 0;
    color: var(--tone);
    font-weight: var(--font-weight-semibold);
}

.toast__close { flex-shrink: 0; }

.toast-enter-active { transition: opacity var(--duration-slow) var(--ease-out), transform var(--duration-slow) var(--ease-out); }
.toast-leave-active { transition: opacity var(--duration) var(--ease-in), transform var(--duration) var(--ease-in); }
.toast-enter-from   { opacity: 0; transform: translateY(12px); }
.toast-leave-to     { opacity: 0; transform: translateY(8px); }
</style>
