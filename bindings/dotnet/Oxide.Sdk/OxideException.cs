namespace Oxide.Sdk;

public sealed class OxideException : Exception
{
    public int Status { get; }

    public OxideException(string message, int status)
        : base(message)
    {
        Status = status;
    }
}
