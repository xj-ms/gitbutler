<script lang="ts">
	import { Icon, SelectItem, type IconName } from "@gitbutler/ui";

	interface Props {
		projectId: string;
		projectTitle: string;
		selected?: boolean;
		highlighted?: boolean;
		hoverIcon?: IconName;
		removable?: boolean;
		onRemove: () => void;
	}

	const {
		projectId,
		projectTitle,
		selected = false,
		highlighted = false,
		hoverIcon,
		removable = false,
		onRemove,
	}: Props = $props();

	function stopKeyboardPropagation(event: KeyboardEvent) {
		event.stopPropagation();
	}
</script>

<div class="project-select-item">
	<SelectItem {selected} {highlighted} {hoverIcon}>{projectTitle}</SelectItem>
	{#if removable}
		<button
			type="button"
			class="remove-project"
			aria-label={`Remove ${projectTitle} from GitButler`}
			data-testid={`remove-project-${projectId}`}
			onclick={(event) => {
				event.stopPropagation();
				onRemove();
			}}
			onkeydown={stopKeyboardPropagation}
		>
			<Icon name="bin" />
		</button>
	{/if}
</div>

<style lang="postcss">
	.project-select-item {
		display: flex;
		position: relative;
		align-items: center;

		:global(.select-button) {
			padding-right: 34px;
		}
	}

	.remove-project {
		display: flex;
		position: absolute;
		right: 4px;
		align-items: center;
		justify-content: center;
		width: 26px;
		height: 26px;
		border-radius: var(--radius-button);
		color: var(--text-2);
		opacity: 0;
		transition: opacity 0.1s ease;

		&:hover,
		&:focus-visible {
			background: var(--hover-bg-1);
			color: var(--fill-danger-bg);
		}
	}

	.project-select-item:hover .remove-project,
	.project-select-item:focus-within .remove-project {
		opacity: 1;
	}
</style>
