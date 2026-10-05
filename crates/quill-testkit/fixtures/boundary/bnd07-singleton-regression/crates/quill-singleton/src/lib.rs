
pub fn bad_data_dir() -> String {

    format!("{:?}", goose_config_global())
}

fn goose_config_global() -> &'static str {
    "Config::global()"
}
