<script lang="ts">
	import { STACK_SERVICE } from "$lib/stacks/stackService.svelte";
	import { inject } from "@gitbutler/core/context";
	import { AsyncButton, Button, Modal } from "@gitbutler/ui";
	import type { ConflictingStack } from "@gitbutler/but-sdk";

	type Props = {
		projectId: string;
		onApplied: () => void | Promise<void>;
	};

	const { projectId, onApplied }: Props = $props();
	const stackService = inject(STACK_SERVICE);

	let modal = $state<Modal>();
	let pending = $state<{ branch: string; prNumber?: number }>();
	let conflicts = $state<ConflictingStack[]>([]);
	let applying = $state(false);

	export async function apply(branch: string, prNumber?: number) {
		await applyBranch(branch, prNumber, []);
	}

	async function applyBranch(
		branch: string,
		prNumber: number | undefined,
		unapplyConflictingStacks: string[],
	) {
		applying = true;
		try {
			const outcome = await stackService.applyWithConfirmation({
				projectId,
				existingBranch: branch,
				prNumber,
				unapplyConflictingStacks,
			});
			if (outcome.status === "conflictAborted") {
				if (outcome.conflictingStacks.length === 0) {
					throw new Error("Branch application was aborted without any stacks to unapply.");
				}
				pending = { branch, prNumber };
				conflicts = outcome.conflictingStacks;
				modal?.show();
				return;
			}
			pending = undefined;
			await modal?.close();
			await onApplied();
		} finally {
			applying = false;
		}
	}

	async function confirm() {
		if (!pending) return;
		await applyBranch(
			pending.branch,
			pending.prNumber,
			conflicts.map((stack) => stack.refName.full),
		);
	}
</script>

<Modal
	bind:this={modal}
	title="Branch conflicts with your workspace"
	type="warning"
	preventCloseOnClickOutside
	onClose={() => {
		pending = undefined;
	}}
>
	<p>
		Applying <strong>{pending?.branch.replace(/^refs\/(heads|remotes)\//, "")}</strong> conflicts with
		the following stacks. Nothing has been unapplied.
	</p>
	<div class="conflicts">
		{#each conflicts as stack (stack.refName.full)}
			<section>
				<h3 class="text-bold">{stack.shortName}</h3>
				<ul>
					{#each stack.paths as path (path)}
						<li><code>{path}</code></li>
					{/each}
				</ul>
			</section>
		{/each}
	</div>
	<p>
		Unapply these stacks to continue applying this branch, or cancel to leave your workspace
		unchanged. Unapplied stacks remain available on the branches page.
	</p>
	{#snippet controls(close)}
		<Button kind="outline" type="reset" disabled={applying} onclick={close}>Cancel</Button>
		<AsyncButton style="pop" action={confirm}>Unapply and continue</AsyncButton>
	{/snippet}
</Modal>

<style lang="postcss">
	.conflicts {
		max-height: 40vh;
		margin: 16px 0;
		overflow: auto;
		overflow-wrap: anywhere;
	}

	section + section {
		margin-top: 12px;
	}

	ul {
		margin-top: 4px;
		padding-left: 20px;
		list-style: disc;
	}
</style>
