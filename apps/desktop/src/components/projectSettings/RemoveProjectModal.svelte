<script lang="ts">
	import { Button, Modal, TestId } from "@gitbutler/ui";

	interface Props {
		projectTitle?: string;
		isDeleting?: boolean;
		onDeleteClicked: () => Promise<void>;
	}

	const { projectTitle = "#", isDeleting, onDeleteClicked }: Props = $props();

	let modal = $state<Modal>();

	export function show() {
		modal?.show();
	}

	export function close() {
		modal?.close();
	}
</script>

<Modal
	bind:this={modal}
	width="small"
	onSubmit={(close) => {
		onDeleteClicked().then(close);
	}}
>
	<div class="remove-project-description">
		<p class="text-14 text-body">
			Are you sure you want to remove
			<span class="text-bold">{projectTitle}</span> from GitButler?
		</p>

		<p class="text-12 text-body details-text">
			When you delete your project from GitButler, your repository doesn't get deleted. It just
			removes the project from the list, keeping your repository safe and easy to access.
		</p>
	</div>

	{#snippet controls(close)}
		<Button
			testId={TestId.ProjectDeleteModalConfirm}
			style="danger"
			kind="outline"
			reversedDirection
			loading={isDeleting}
			icon="bin"
			type="submit"
		>
			Remove
		</Button>
		<Button style="pop" onclick={close}>Cancel</Button>
	{/snippet}
</Modal>

<style lang="postcss">
	.remove-project-description {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.details-text {
		opacity: 0.5;
	}
</style>
