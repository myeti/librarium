<template>
    <div class="activity">
        <button
            ref="trigger"
            type="button"
            class="nav-item activity__trigger"
            :class="{ '-open': open }"
            aria-haspopup="dialog"
            :aria-expanded="open ? 'true' : 'false'"
            :aria-controls="open ? popoverId : undefined"
            :aria-label="triggerLabel"
            @click="toggle"
        >
            <vue-feather type="activity" size="18" aria-hidden="true" />
            <span>{{ $t('activity.button') }}</span>
            <vue-feather v-if="failedCount > 0" type="alert-circle" size="15" class="activity__failed" aria-hidden="true" />
            <span v-else-if="runningCount > 0" class="nav-item_count -accent" aria-hidden="true">{{ runningCount }}</span>
        </button>

        <Teleport to="body">
            <section
                v-if="open"
                :id="popoverId"
                ref="popover"
                class="activity__popover"
                role="dialog"
                :aria-labelledby="`${popoverId}-title`"
                tabindex="-1"
                :style="position"
                @keydown.esc.prevent.stop="close(true)"
            >
                <header class="activity__header">
                    <h2 :id="`${popoverId}-title`" class="activity__title">{{ $t('activity.title') }}</h2>
                    <BaseButton
                        v-if="finishedCount > 0"
                        variant="ghost"
                        size="sm"
                        icon="check-square"
                        @click="deviceStore.clearFinished()"
                    >
                        {{ $t('activity.clearFinished') }}
                    </BaseButton>
                    <IconButton icon="x" size="sm" :label="$t('common.close')" @click="close(true)" />
                </header>

                <EmptyState
                    v-if="jobs.length === 0"
                    icon="send"
                    :icon-size="28"
                    :title="$t('activity.empty')"
                    :description="$t('activity.emptyBody')"
                />

                <ul v-else class="activity__list">
                    <li v-for="job in jobs" :key="job.id" class="activity__job" :data-status="job.status">
                        <span class="activity__icon" aria-hidden="true">
                            <Spinner v-if="isRunning(job)" :size="14" />
                            <vue-feather v-else :type="STATUS_ICONS[job.status]" size="14" />
                        </span>
                        <div class="activity__body">
                            <p class="activity__name" :title="jobTitle(job)">{{ jobTitle(job) }}</p>
                            <p class="activity__status">
                                {{ statusLabel(job) }}
                                <template v-if="job.direction === 'from-device'"> · {{ $t('activity.fromDevice') }}</template>
                                <template v-else-if="job.needsConversion && job.book"> · {{ job.book.format.toUpperCase() }} → {{ job.targetFormat.toUpperCase() }}</template>
                            </p>
                            <ProgressBar
                                v-if="isRunning(job)"
                                :value="job.status === 'queued' ? null : job.progress"
                                :label="`${jobTitle(job)}: ${statusLabel(job)}`"
                            />
                            <p v-if="job.errorMessage" class="activity__error">{{ job.errorMessage }}</p>
                        </div>
                        <!-- The backend can stop a job until its copy starts (docs/IPC.md). -->
                        <IconButton
                            v-if="canCancel(job)"
                            icon="x-circle"
                            size="sm"
                            :label="`${$t('transfer.cancel')}: ${jobTitle(job)}`"
                            :disabled="job.cancelRequested"
                            @click="deviceStore.cancelTransfer(job.id)"
                        />
                    </li>
                </ul>
            </section>
        </Teleport>
    </div>
</template>

<script lang="ts">
    import { defineComponent } from 'vue'
    import { mapStores } from 'pinia'
    import { isSettled, useDeviceStore } from '@/stores/device'
    import { displayTitle } from '@/utils/bookDisplay'
    import { uniqueId } from '@/utils/dom'
    import BaseButton from '@/components/ui/BaseButton.vue'
    import IconButton from '@/components/ui/IconButton.vue'
    import EmptyState from '@/components/ui/EmptyState.vue'
    import ProgressBar from '@/components/ui/ProgressBar.vue'
    import Spinner from '@/components/ui/Spinner.vue'
    import type { TransferJob, TransferStatus } from '@/types'

    /** The icon of a job that isn't running (a running one shows the spinner). */
    const STATUS_ICONS: Record<TransferStatus, string> = {
        queued: 'clock',
        converting: 'loader',
        transferring: 'loader',
        done: 'check-circle',
        error: 'alert-circle',
        cancelled: 'slash',
    }

    /** Space kept between the popover and the window's edges. */
    const MARGIN = 12

    /**
     * The sidebar's Activity item and its popover (it replaces the old
     * Transfers page): the transfers to and from the e-reader, newest last,
     * each with its state in words, a progress bar while it runs, the
     * reason when it failed, and Cancel while the backend can still stop it.
     * "Clear finished" forgets the jobs that are over.
     *
     * The item shows how many transfers are running, or an alert icon when
     * one failed (its name says so for screen readers).
     *
     * The popover is a non-modal dialog next to the item: it takes the
     * focus when opened, and closes with Escape (focus back on the item),
     * its × button, or a click outside it.
     */
    export default defineComponent({
        name: 'ActivityPopover',

        components: { BaseButton, IconButton, EmptyState, ProgressBar, Spinner },

        data() {
            return {
                open: false,
                popoverId: uniqueId('activity'),
                position: {},
            }
        },

        computed: {
            ...mapStores(useDeviceStore),

            jobs(): TransferJob[] { return this.deviceStore.transferJobs },
            runningCount(): number { return this.deviceStore.activeJobs.length },
            failedCount(): number { return this.deviceStore.failedJobs.length },
            finishedCount(): number { return this.deviceStore.finishedJobs.length },

            /** "Activity, 2 transfers in progress, 1 transfer failed". */
            triggerLabel(): string {
                const parts = [this.$t('activity.button')]
                if (this.runningCount > 0) { parts.push(this.$t('activity.running', { n: this.runningCount })) }
                if (this.failedCount > 0) { parts.push(this.$t('activity.failed', { n: this.failedCount })) }
                return parts.join(', ')
            },

            STATUS_ICONS: () => STATUS_ICONS,
        },

        beforeUnmount() {
            this.stopListening()
        },

        methods: {
            toggle() {
                if (this.open) {
                    this.close(false)
                } else {
                    void this.show()
                }
            },

            async show() {
                this.place()
                this.open = true
                await this.$nextTick()
                ;(this.$refs.popover as HTMLElement | undefined)?.focus()
                document.addEventListener('pointerdown', this.onDocumentPointerdown, true)
                window.addEventListener('resize', this.place)
            },

            /** Closes the popover; `restoreFocus` puts the focus back on the item (Escape, ×). */
            close(restoreFocus: boolean) {
                if (!this.open) { return }
                this.open = false
                this.stopListening()
                if (restoreFocus) { (this.$refs.trigger as HTMLElement | undefined)?.focus() }
            },

            stopListening() {
                document.removeEventListener('pointerdown', this.onDocumentPointerdown, true)
                window.removeEventListener('resize', this.place)
            },

            /** Next to the item, inside the window. */
            place() {
                const rect = (this.$refs.trigger as HTMLElement | undefined)?.getBoundingClientRect()
                if (!rect) { return }
                const top = Math.max(MARGIN, Math.min(rect.top, window.innerHeight - 240))
                this.position = {
                    left: `${Math.round(rect.right + MARGIN)}px`,
                    top: `${Math.round(top)}px`,
                    maxHeight: `${Math.max(200, window.innerHeight - top - MARGIN)}px`,
                }
            },

            onDocumentPointerdown(e: PointerEvent) {
                const target = e.target as Node | null
                const popover = this.$refs.popover as HTMLElement | undefined
                const trigger = this.$refs.trigger as HTMLElement | undefined
                // A click on the item itself toggles it (handled by its click).
                if (target && (popover?.contains(target) || trigger?.contains(target))) { return }
                this.close(false)
            },

            isRunning(job: TransferJob): boolean {
                return !isSettled(job.status)
            },

            canCancel(job: TransferJob): boolean {
                return job.direction === 'to-device' && (job.status === 'queued' || job.status === 'converting')
            },

            jobTitle(job: TransferJob): string {
                if (job.direction === 'from-device') { return job.deviceTitle ?? '' }
                return job.book ? displayTitle(job.book, this.$t) : ''
            },

            statusLabel(job: TransferJob): string {
                if (job.cancelRequested) { return this.$t('transfer.cancelling') }
                if (job.direction === 'from-device' && job.status === 'transferring') { return this.$t('transfer.importing') }
                return this.$t(`transfer.${job.status}`)
            },
        },
    })
</script>

<style lang="scss" scoped>
.activity__trigger {
    width: 100%;
    font: inherit;
    text-align: left;
    cursor: pointer;

    &.-open {
        background: color-mix(in srgb, var(--text-secondary) 12%, transparent);
        color: var(--text-primary);
        border-color: var(--border);
    }
}

.activity__failed {
    margin-left: auto;
    color: var(--danger);
    flex-shrink: 0;
}

.activity__popover {
    position: fixed;
    z-index: var(--z-popover);
    width: 340px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow-popover);
    outline: none;

    &:focus-visible { box-shadow: var(--shadow-popover), 0 0 0 var(--focus-ring-width) var(--focus-ring-color); }
}

.activity__header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--border);
}

.activity__title {
    flex: 1;
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    color: var(--text-primary);
}

.activity__list {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
}

.activity__job {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border);

    &:last-child { border-bottom: none; }
}

.activity__icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    flex-shrink: 0;
    border-radius: 50%;
    color: var(--text-muted);
    background: var(--bg-hover);

    [data-status='converting'] &,
    [data-status='transferring'] & { color: var(--accent-text); background: var(--accent-muted); }
    [data-status='done'] & { color: var(--success); background: var(--success-muted); }
    [data-status='error'] & { color: var(--danger); background: var(--danger-muted); }
}

.activity__body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
}

.activity__name {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}

.activity__status {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--text-secondary);
}

.activity__error {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--danger);
    overflow-wrap: anywhere;
}
</style>
