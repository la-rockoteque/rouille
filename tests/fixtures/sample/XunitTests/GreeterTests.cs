namespace XunitTests;

public class GreeterTests
{
    [Fact]
    public void Greets() => Assert.Equal("Hello, Ada", Shared.Greeter.Greet("Ada"));
}
