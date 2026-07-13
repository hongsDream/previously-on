#[test]
fn gitguardian_exceptions_are_exact_occurrence_hashes_only() {
    let config = include_str!("../.gitguardian.yaml");

    assert!(config.lines().any(|line| line.trim() == "version: 2"));
    assert!(
        !config.contains("ignored_paths"),
        "GitGuardian must continue scanning every repository path"
    );
    assert!(
        !config.contains("ignored_detectors"),
        "GitGuardian detectors must not be disabled repository-wide"
    );

    let matches = config
        .lines()
        .filter_map(|line| line.trim().strip_prefix("match: "))
        .collect::<Vec<_>>();
    assert_eq!(
        matches,
        [
            "66cd5cc17d3c0dbe1a85d0a58c76faa02699cf0d5568a5baa6a60510f4026614",
            "35106a3c612c18c3956a6cccff0ba7dba932fd85a6930824a10fbba1c4c64550",
            "536b9710915ea7d9039854fb792353a003d08327c88aba38e0c83a0b9de1a705",
            "27add1d97158728485b65b572fd09de22761f5c35b8bd450e48cee53d2b34db5",
        ]
    );
    assert!(matches.iter().all(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }));
}
