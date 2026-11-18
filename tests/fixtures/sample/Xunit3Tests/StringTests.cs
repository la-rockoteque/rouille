namespace Xunit3Tests;

public class StringTests
{
    [Fact]
    public void Concatenates() => Assert.Equal("ab", "a" + "b");

    [Fact]
    public async Task Waits()
    {
        await Task.Delay(200);
        Assert.True(true);
    }

    [Fact]
    public void FailsOnDemand() =>
        Assert.True(Environment.GetEnvironmentVariable("ROUILLE_FIXTURE_FAIL") != "1", "fixture failure requested");
}
