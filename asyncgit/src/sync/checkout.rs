use super::{
	branch::checkout_remote_branch, checkout_branch, stash_drop,
	stash_save, BranchInfo, RepoPath,
};
use crate::{
	error::{Error, Result},
	sync::{stash::stash_apply_reinstate_index, status::discard_status},
};

/// Strategy used when checking out a branch with local changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutMethod {
	/// Keep local changes in the working tree and let libgit2 reject
	/// conflicting checkouts.
	KeepLocalChanges,
	/// Temporarily stash local changes, checkout the branch, then restore the
	/// stash including its index state.
	StashAndReapply,
	/// Discard all local changes before checkout.
	DiscardLocalChanges,
}

/// Checkout `branch` using the requested local-change strategy.
pub fn checkout_with_method(
	repo_path: &RepoPath,
	branch: &BranchInfo,
	method: CheckoutMethod,
) -> Result<()> {
	match method {
		CheckoutMethod::KeepLocalChanges => checkout(repo_path, branch),
		CheckoutMethod::DiscardLocalChanges => {
			discard_status(repo_path)?;
			checkout(repo_path, branch)
		}
		CheckoutMethod::StashAndReapply => {
			checkout_stash_and_reapply(repo_path, branch)
		}
	}
}

fn checkout(repo_path: &RepoPath, branch: &BranchInfo) -> Result<()> {
	if branch.is_local() {
		checkout_branch(repo_path, &branch.name)
	} else {
		checkout_remote_branch(repo_path, branch)
	}
}

fn checkout_stash_and_reapply(
	repo_path: &RepoPath,
	branch: &BranchInfo,
) -> Result<()> {
	let stash_id = stash_save(repo_path, None, true, false)?;

	if let Err(checkout_error) = checkout(repo_path, branch) {
		if stash_apply_reinstate_index(repo_path, stash_id, false).is_ok() {
			stash_drop(repo_path, stash_id)?;
			return Err(checkout_error);
		}

		return Err(Error::Generic(format!(
			"checkout failed and local changes could not be restored; changes are preserved in stash {}",
			stash_id.get_short_string()
		)));
	}

	if let Err(error) =
		stash_apply_reinstate_index(repo_path, stash_id, false)
	{
		return Err(Error::Generic(format!(
			"switched branches, but local changes could not be reapplied: {error}; changes are preserved in stash {}",
			stash_id.get_short_string()
		)));
	}

	stash_drop(repo_path, stash_id)?;

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::sync::{
		branch::{create_branch, get_branches_info},
		get_stashes, stage_add_file,
		status::{get_status, StatusType},
		tests::{repo_init, write_commit_file},
		utils::{repo_read_file, repo_write_file},
	};
	use std::path::Path;

	fn branch(repo_path: &RepoPath, name: &str) -> BranchInfo {
		get_branches_info(repo_path, true)
			.unwrap()
			.into_iter()
			.find(|branch| branch.name == name)
			.unwrap()
	}

	#[test]
	fn test_stash_and_reapply_preserves_staged_and_unstaged_changes(
	) -> Result<()> {
		let (_td, repo) = repo_init()?;
		let root = repo.path().parent().unwrap();
		let repo_path: &RepoPath =
			&root.as_os_str().to_str().unwrap().into();

		write_commit_file(&repo, "test.txt", "base\n", "base");
		create_branch(repo_path, "target")?;

		repo_write_file(&repo, "test.txt", "staged\n")?;
		stage_add_file(repo_path, Path::new("test.txt"))?;
		repo_write_file(&repo, "test.txt", "staged\nunstaged\n")?;

		checkout_with_method(
			repo_path,
			&branch(repo_path, "target"),
			CheckoutMethod::StashAndReapply,
		)?;

		assert_eq!(
			get_status(repo_path, StatusType::Stage, None)?.len(),
			1
		);
		assert_eq!(
			get_status(repo_path, StatusType::WorkingDir, None)?.len(),
			1
		);
		assert_eq!(
			repo_read_file(&repo, "test.txt")?,
			"staged\nunstaged\n"
		);
		assert!(get_stashes(repo_path)?.is_empty());

		Ok(())
	}

	#[test]
	fn test_stash_and_reapply_preserves_existing_stashes() -> Result<()> {
		let (_td, repo) = repo_init()?;
		let root = repo.path().parent().unwrap();
		let repo_path: &RepoPath =
			&root.as_os_str().to_str().unwrap().into();

		write_commit_file(&repo, "test.txt", "base\n", "base");
		create_branch(repo_path, "target")?;

		repo_write_file(&repo, "old.txt", "existing stash")?;
		let existing = stash_save(repo_path, Some("existing"), true, false)?;

		repo_write_file(&repo, "test.txt", "local\n")?;
		checkout_with_method(
			repo_path,
			&branch(repo_path, "target"),
			CheckoutMethod::StashAndReapply,
		)?;

		assert_eq!(get_stashes(repo_path)?, vec![existing]);
		assert_eq!(repo_read_file(&repo, "test.txt")?, "local\n");

		Ok(())
	}
}
