use agentz_core::plan::{Dag, DagError, Objective, Plan, Step, StepKind};

#[test]
fn topological_order_is_deterministic() {
    let compile = Step::new("compile", "compile tree", StepKind::Compile);
    let install = Step::new(
        "install",
        "install links",
        StepKind::Install {
            project_key: "demo".into(),
        },
    );
    let commit = Step::new(
        "commit",
        "commit changes",
        StepKind::Shell {
            command: "git commit".into(),
        },
    );

    let mut plan = Plan::new(Objective::new("ship"));
    plan.dag.add(install.clone()).unwrap();
    plan.dag.add(commit.clone()).unwrap();
    plan.dag.add(compile.clone()).unwrap();
    plan.dag.edge(&compile.id, &install.id).unwrap();
    plan.dag.edge(&install.id, &commit.id).unwrap();

    let order = plan.topo().unwrap();
    let ids: Vec<_> = order.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["compile", "install", "commit"]);
}

#[test]
fn cycle_is_rejected() {
    let mut dag = Dag::new();
    let a = Step::new("a", "a", StepKind::Noop);
    let b = Step::new("b", "b", StepKind::Noop);
    dag.add(a.clone()).unwrap();
    dag.add(b.clone()).unwrap();
    dag.edge(&a.id, &b.id).unwrap();
    dag.edge(&b.id, &a.id).unwrap();
    assert!(matches!(dag.topo(), Err(DagError::Cycle(_))));
}

#[test]
fn duplicate_step_id_is_rejected() {
    let mut dag = Dag::new();
    dag.add(Step::new("a", "a", StepKind::Noop)).unwrap();
    let err = dag
        .add(Step::new("a", "again", StepKind::Noop))
        .unwrap_err();
    assert!(matches!(err, DagError::Duplicate(_)));
}
