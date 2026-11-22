namespace shapes {
class Circle {
public:
    double area() const;

private:
    double radius_;
};

double Circle::area() const {
    return 3.14 * radius_ * radius_;
}
}
