struct Vec {
    int x;
    Vec operator+(const Vec& other) const;
    bool operator==(const Vec& other) const = default;
};
