namespace XunitTests;

public class MathTests
{
    [Fact]
    public void Adds() => Assert.Equal(4, 2 + 2);

    [Theory]
    [InlineData(1, 1)]
    [InlineData(2, 4)]
    [InlineData(3, 9)]
    public void Squares(int n, int expected) => Assert.Equal(expected, n * n);

    [Fact]
    public void FailsOnDemand() =>
        Assert.True(Environment.GetEnvironmentVariable("ROUILLE_FIXTURE_FAIL") != "1", "fixture failure requested");
}
