namespace WellfriendPdf;

public sealed class WellfriendPdfException : Exception
{
    public int Status { get; }

    public WellfriendPdfException(string message, int status)
        : base(message)
    {
        Status = status;
    }
}
