# This thing is actually decent

## Usage
Example:
```
./cantelope --dbc fs.dbc --candump -i realdata.log --cache-ms 10 --output testi.parquet
```
You can also use `--stdin` or  `--socket` instead of `--candump`.

There's single letter versions of the arguments, but I don't remember them. Check `src/args.rs`. It's quite readable.
