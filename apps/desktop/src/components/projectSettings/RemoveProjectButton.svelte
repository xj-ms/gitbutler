<script lang="ts">
	import RemoveProjectModal from "$components/projectSettings/RemoveProjectModal.svelte";
	import { Button, TestId } from "@gitbutler/ui";

	interface Props {
		projectTitle?: string;
		isDeleting?: boolean;
		noModal?: boolean;
		outlineStyle?: boolean;
		onDeleteClicked: () => Promise<void>;
	}

	const {
		projectTitle = "#",
		isDeleting,
		noModal,
		outlineStyle,
		onDeleteClicked,
	}: Props = $props();

	export function show() {
		removeProjectModal?.show();
	}
	export function close() {
		removeProjectModal?.close();
	}

	function handleClick() {
		if (noModal) {
			onDeleteClicked();
		} else {
			removeProjectModal?.show();
		}
	}

	let removeProjectModal = $state<RemoveProjectModal>();
</script>

<Button
	testId={TestId.ProjectDeleteButton}
	style="danger"
	kind={outlineStyle ? "outline" : "solid"}
	icon="bin"
	reversedDirection
	onclick={handleClick}
>
	Remove project…
</Button>

<RemoveProjectModal bind:this={removeProjectModal} {projectTitle} {isDeleting} {onDeleteClicked} />
