# You can load any libraries you want, but don't load anything on execution of process_numbers()
# process_numbers() should be optimal and hopefully fairly minimal. Executions need to be less
# than 8ms to avoid overruns.


class CantelopeExtension:
    def __init__(self, all_keys: list[str]):
        self.all_keys = all_keys
        # you get a list of all signals in the DBC, plus "Time_ms".
        print(all_keys)

    @classmethod
    def create_and_initialize(
        cls, all_keys: list[str]
    ) -> tuple["CantelopeExtension", list[str]]:
        instance = cls(
            all_keys
        )  # this line is just calling the __init__. You can change this if you like
        desired_keys = all_keys  # return a list of str of signals you will actually be using. There is a
        # somewhat significant performance implication, so don't just request all
        # of them.
        return instance, desired_keys

    def process_numbers(self, numbers: list[float | None]) -> list[tuple[str, float]]:
        # Filter out None values
        valid_numbers = [n for n in numbers if n is not None]

        avg = sum(valid_numbers) / (len(valid_numbers) + 0.0001)
        return [
            ("Average", avg)
        ]  # return a list of your outputs. These can change each execution and only
        # go to the plot subsystem, so I recommend building this class in a way
        # where your analysis can also be called on stored parquet files, but
        # if you don't cantelope also provides that functionality, it's just uggo
