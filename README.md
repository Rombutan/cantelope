# This thing is actually decent

## Usage
Example:
```
./cantelope --dbc fs.dbc --candump -i realdata.log --cache-ms 10 --output testi.parquet
```
You can also use `--stdin` or  `--socket` instead of `--candump`.

If you don't pass `--output`, cantelope won't store values. This is useful for using the live plotting function. Memory usage should be near zero under this circumstance.

There's single letter versions of the arguments, but I don't remember them. Check `src/args.rs`. It's quite readable.

`--stdin` and `--candump` expect line seperated frames in the following format `(time in seconds) interface id_in_hex#data_in_hex` Ex:
```
(1759876075.171400) can0 288#8A2C642B00000000
```
You can produce these with `candump -ta -n 0 can0` for stdout output or `candump -L` for log file output.

## Remote mode
You can add `--remote` and specify `ip:port` as your input `-i` argument, to connect to a TCP server.

Conveniently available is the sender binary which retransmits packets in the appropriate format. Ex:
```
./sender vcan0 2129"
```

## Build notes
- If you're on linux, build with `--features socket` so you can use SocketCan interfaces.
- If you wanna cross compile for windows, google it.
