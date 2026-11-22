class Service:
    class Config:
        def validate(self):
            return True

    def start(self):
        def log(message):
            print(message)

        log("start")


def main():
    Service().start()
