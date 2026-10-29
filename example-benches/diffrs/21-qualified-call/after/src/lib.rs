fn boot() {
    let timeout = config_loader::read_config_value(default_timeout_ms);
    let retries = config_loader::read_config_value(max_retry_count);
    let endpoint = config_loader::read_config_value(service_endpoint_url);
    let workers = config_loader::read_config_value(worker_pool_size);
    let buffer = config_loader::read_config_value(shared_buffer_bytes);
    let verbose = config_loader::read_config_value(verbose_logging_flag);
}
