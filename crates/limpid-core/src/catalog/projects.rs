//! Build output inside the person's own projects.
//!
//! Usually the largest thing on a developer's machine, and invisible to
//! every other scanner, because no list of places can name it. What makes a
//! directory count is in [`crate::project`]; the guard asks the same
//! question again at the moment of removal.
//!
//! Every project is listed, however recently it was worked on. How recently
//! decides only whether it starts ticked. Hiding the build output of the
//! project someone is working on would hide what is, as often as not, the
//! largest single thing on the disk — and the storage view would show it
//! anyway, unexplained.

use std::time::{Duration, SystemTime};

use super::Context;
use crate::model::{Category, Kind, Risk, Target};
use crate::project::{self, BuildOutput};

/// Untouched this long, and removing the build output costs nothing that
/// would not have been spent anyway.
///
/// Rust ships a compiler every six weeks and a new one rebuilds `target/`
/// from scratch whatever is in it; a quarter-old `node_modules` is one
/// `npm install` from being brought up to date regardless.
const STALE: Duration = Duration::from_secs(90 * 86_400);

/// Find build output in every project under the home directory.
pub fn scan(context: &Context) -> Category {
    scan_at(context, SystemTime::now())
}

/// The same, as of `now`, so the tests can make a project old without
/// waiting three months.
fn scan_at(context: &Context, now: SystemTime) -> Category {
    let mut category = Category::new(
        "Projects",
        "Build output and installed packages inside your projects. One command \
         brings each of them back.",
    );

    for output in project::discover(&context.roots.home, &context.config.exclusions) {
        let age = project::last_touched(&output).and_then(|when| now.duration_since(when).ok());
        category.targets.push(target(context, &output, age));
    }

    context.measure_all(category)
}

/// One project's build output, as a finding.
fn target(context: &Context, output: &BuildOutput, age: Option<Duration>) -> Target {
    // Unknown counts as recent. A clock that went backwards, or a
    // filesystem that would not say, must not make anything look abandoned.
    let stale = age.is_some_and(|age| age >= STALE);
    let risk = if stale { Risk::Safe } else { Risk::Review };

    let when = match age {
        Some(age) if stale => format!("Untouched for {}", span(age)),
        Some(age) if age < Duration::from_secs(86_400) => "Worked on today".to_owned(),
        Some(age) => format!("Last worked on {} ago", span(age)),
        None => "When it was last worked on is unknown".to_owned(),
    };

    // The path itself, from the home directory: two projects are often
    // called the same thing, and this is exactly what goes.
    let name = match output.directory.strip_prefix(&context.roots.home) {
        Ok(relative) => format!("~/{}", relative.display()),
        Err(_) => output.directory.display().to_string(),
    };

    Target::new(name, Kind::BuildArtifact, risk)
        .detail(format!(
            "{}. {when}; {}.",
            output.ecosystem.describe(),
            output.ecosystem.restored_by(),
        ))
        .path(&output.directory)
        // A build running in it right now would fail half-way and leave
        // the next one confused. Checked when removing, not when scanning.
        .requires_idle(&output.directory)
        .build_output()
}

/// A length of time in the largest unit that reads naturally.
fn span(age: Duration) -> String {
    let days = age.as_secs() / 86_400;
    let (count, unit) = match days {
        0..=13 => (days.max(1), "day"),
        14..=59 => (days / 7, "week"),
        60..=729 => (days / 30, "month"),
        _ => (days / 365, "year"),
    };
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Exclusions};
    use crate::guard::Permission;
    use crate::paths::Roots;
    use crate::plan::{Disposal, Plan};
    use crate::project::fixture::{cargo_project, npm_project};

    const DAY: Duration = Duration::from_secs(86_400);

    fn fixture() -> (tempfile::TempDir, Context) {
        let directory = tempfile::tempdir().unwrap();
        let context = Context::with_roots(Roots::under(directory.path()));
        std::fs::create_dir_all(&context.roots.home).unwrap();
        (directory, context)
    }

    #[test]
    fn a_project_untouched_for_months_is_offered_as_safe() {
        let (_fixture, context) = fixture();
        cargo_project(&context.roots.home("Work/old"), 4096);

        let category = scan_at(&context, SystemTime::now() + 200 * DAY);

        let target = &category.targets[0];
        assert_eq!(target.name, "~/Work/old/target");
        assert_eq!(target.risk, Risk::Safe);
        assert!(
            target.detail.contains("Untouched for 6 months"),
            "{}",
            target.detail
        );
        assert!(target.size.on_disk >= 4096);
    }

    #[test]
    fn a_project_worked_on_recently_is_listed_but_needs_a_decision() {
        let (_fixture, context) = fixture();
        cargo_project(&context.roots.home("Work/limpid"), 4096);

        let category = scan_at(&context, SystemTime::now());

        let target = &category.targets[0];
        assert_eq!(target.risk, Risk::Review);
        assert!(
            target.detail.contains("Worked on today"),
            "{}",
            target.detail
        );
        assert!(
            target.detail.contains("the next build starts from scratch"),
            "{}",
            target.detail
        );
    }

    #[test]
    fn a_clock_that_went_backwards_makes_nothing_look_abandoned() {
        let (_fixture, context) = fixture();
        cargo_project(&context.roots.home("Work/x"), 10);

        // Every file is newer than "now".
        let category = scan_at(&context, SystemTime::now() - 30 * DAY);

        assert_eq!(category.targets[0].risk, Risk::Review);
    }

    #[test]
    fn build_output_is_deleted_under_its_own_permission_and_waits_for_a_build_to_finish() {
        let (_fixture, context) = fixture();
        let modules = npm_project(&context.roots.home("Work/site"), 10);

        let category = scan_at(&context, SystemTime::now() + 200 * DAY);
        let plan = Plan::from_targets(&category.targets);

        let item = &plan.items[0];
        assert_eq!(item.permission, Permission::BuildOutput);
        // Regenerable, and trashing it would free nothing.
        assert_eq!(item.disposal, Disposal::Delete);
        assert_eq!(item.requires_idle.as_deref(), Some(modules.as_path()));
    }

    #[test]
    fn an_excluded_project_is_not_offered() {
        let directory = tempfile::tempdir().unwrap();
        let roots = Roots::under(directory.path());
        cargo_project(&roots.home("Work/keep"), 10);
        let context = Context::with_config(
            roots.clone(),
            Config {
                exclusions: Exclusions::new([roots.home("Work/keep")]),
                ..Config::default()
            },
        );

        assert!(scan_at(&context, SystemTime::now()).targets.is_empty());
    }

    #[test]
    fn cleaning_empties_the_build_output_and_touches_nothing_else_in_the_project() {
        let (_fixture, context) = fixture();
        let project = context.roots.home("Work/old");
        let target = cargo_project(&project, 4096);
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::write(project.join("src/main.rs"), "fn main() {}").unwrap();

        let category = scan_at(&context, SystemTime::now() + 200 * DAY);
        let plan = Plan::from_targets(&category.targets);
        let outcome = crate::execute::Executor::applying(&context.roots).run(&plan);

        assert!(outcome.is_clean(), "{:?}", outcome.problems);
        assert!(outcome.reclaimed.on_disk >= 4096);
        // Emptied, not removed: tools expect it to exist.
        assert!(target.is_dir());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert!(project.join("Cargo.toml").is_file());
        assert!(project.join("src/main.rs").is_file());
    }

    #[test]
    fn a_project_that_stopped_being_one_after_the_scan_is_left_alone() {
        let (_fixture, context) = fixture();
        let project = context.roots.home("Work/old");
        let target = cargo_project(&project, 4096);

        let category = scan_at(&context, SystemTime::now() + 200 * DAY);
        let plan = Plan::from_targets(&category.targets);
        std::fs::remove_file(project.join("Cargo.toml")).unwrap();
        let outcome = crate::execute::Executor::applying(&context.roots).run(&plan);

        assert!(matches!(
            outcome.problems.as_slice(),
            [crate::execute::Problem::Refused(
                crate::guard::Refusal::NotBuildOutput(_)
            )]
        ));
        assert!(target.join("debug/deps/libx.rlib").is_file());
    }

    #[test]
    fn a_length_of_time_reads_the_way_a_person_would_say_it() {
        assert_eq!(span(DAY / 2), "1 day");
        assert_eq!(span(3 * DAY), "3 days");
        assert_eq!(span(20 * DAY), "2 weeks");
        assert_eq!(span(200 * DAY), "6 months");
        assert_eq!(span(800 * DAY), "2 years");
    }
}
