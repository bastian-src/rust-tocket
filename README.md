# rust-tocket

rust-tocket is a simple Rust TCP-socket tool to inspect TCP transmission params:

* creates a TCP server TCP server socket (default port: 9393)
* a user can run `wget` on the port and receives dummy data for 10 seconds
* while the user retrieves data, rust-tocket logs TCP info like _cwnd_ and _rtt_ in `.logs/run_<date>.jsonl`

You can use the `scripts/visualize_tcp_params.py` script to plot your transmissions:

```
python3 scripts/visualize_tcp_params.py --path "./logs/<your log>.jsonl" diashow
```

