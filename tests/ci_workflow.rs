//! Keeps the CI gate's skip list in step with the jobs it may skip.
//!
//! The gate in `.github/workflows/ci.yml` is allowed to skip the post-merge run
//! when the pull request already proved those jobs. It proves it by requiring
//! every name in `REQUIRED` to be present on the pull request head with a
//! `success` conclusion — so a job missing from that list is a job a pull
//! request can drop, rename or condition away while still looking tested, and
//! the push run then skips a suite that never ran.
//!
//! `ci.yml` asks a human to keep the two in step in a comment. This is that
//! comment as a test: adding a job without adding its name fails here, and so
//! does a name that no longer matches a job. It caught exactly this when the
//! `OpenAPI contracts` job was added to the workflow.

/// The workflow, compiled in so a rename fails the build rather than the test.
const WORKFLOW: &str = include_str!("../.github/workflows/ci.yml");

/// One job: the key under `jobs:`, its `name:`, and whether it hangs off the gate.
struct Job {
    key: String,
    name: Option<String>,
    gated: bool,
}

/// The jobs declared under `jobs:`, in file order.
///
/// A hand-rolled scan rather than a YAML parser: the file is machine-written,
/// the shape is fixed, and the two things read here — a job key at two spaces
/// and its `name:`/`needs:` at four — are unambiguous in it. Anything it cannot
/// read fails the assertions below rather than passing quietly.
fn jobs() -> Vec<Job> {
    let mut found: Vec<Job> = Vec::new();
    let mut in_jobs = false;

    for line in WORKFLOW.lines() {
        if line.starts_with("jobs:") {
            in_jobs = true;
            continue;
        }
        if !in_jobs {
            continue;
        }
        // A top-level key after `jobs:` ends the block.
        if !line.starts_with(' ') && !line.trim().is_empty() {
            break;
        }
        // `  <key>:` with nothing else on the line starts a job.
        if let Some(rest) = line.strip_prefix("  ") {
            if !rest.starts_with(' ') && rest.ends_with(':') && rest.len() > 1 {
                found.push(Job {
                    key: rest.trim_end_matches(':').to_string(),
                    name: None,
                    gated: false,
                });
                continue;
            }
        }
        let Some(job) = found.last_mut() else {
            continue;
        };
        if let Some(value) = line.strip_prefix("    name:") {
            job.name = Some(value.trim().trim_matches('"').to_string());
        } else if let Some(value) = line.strip_prefix("    needs:") {
            job.gated = value.trim() == "gate";
        }
    }

    found
}

/// The names in the gate's `REQUIRED` environment variable.
fn required() -> Vec<String> {
    let line = WORKFLOW
        .lines()
        .find(|line| line.trim_start().starts_with("REQUIRED:"))
        .expect("ci.yml has no REQUIRED list for the gate");
    line.split_once("REQUIRED:")
        .expect("REQUIRED prefix")
        .1
        .split(',')
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

#[test]
fn the_workflow_is_readable() {
    // A parser that stopped matching would make the comparisons below vacuous.
    let jobs = jobs();
    assert!(
        jobs.len() >= 4,
        "only {} jobs were parsed out of ci.yml; the parser is probably broken",
        jobs.len()
    );
    assert!(
        jobs.iter().filter(|job| job.gated).count() >= 3,
        "no gated jobs were parsed out of ci.yml; the parser is probably broken"
    );
    assert!(!required().is_empty(), "the gate's REQUIRED list is empty");
}

#[test]
fn every_gated_job_is_in_the_gates_skip_list() {
    let required = required();
    let mut missing: Vec<String> = jobs()
        .iter()
        .filter(|job| job.gated)
        .filter(|job| {
            job.name
                .as_ref()
                .is_none_or(|name| !required.contains(name))
        })
        .map(|job| match &job.name {
            Some(name) => format!("{} (job `{}`)", name, job.key),
            None => format!("job `{}` has no name", job.key),
        })
        .collect();
    missing.sort();

    assert!(
        missing.is_empty(),
        "these jobs hang off the gate but are not in its REQUIRED list, so a pull \
         request could drop or rename them and still look tested: {missing:?}\n\
         Add each name to REQUIRED in .github/workflows/ci.yml."
    );
}

#[test]
fn the_gates_skip_list_names_real_jobs() {
    let names: Vec<String> = jobs().iter().filter_map(|job| job.name.clone()).collect();
    let unknown: Vec<String> = required()
        .into_iter()
        .filter(|name| !names.contains(name))
        .collect();

    assert!(
        unknown.is_empty(),
        "the gate's REQUIRED list names jobs that do not exist, so it would run \
         the suite on every push: {unknown:?}\n\
         Every entry has to match a job's `name:` in .github/workflows/ci.yml."
    );
}
