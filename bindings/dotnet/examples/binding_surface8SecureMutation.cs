using WellfriendPdf;

if (args.Length != 1)
{
    Console.Error.WriteLine("usage: SecureMutationSecureMutation input.pdf");
    return 2;
}

using var document = WellfriendDocument.Open(File.ReadAllBytes(args[0]));
Console.WriteLine(document.SecureMutationReportJson());
Console.WriteLine(document.AssociatedFilesReportJson());
Console.WriteLine(document.EditPolicyReportJson("attachment_remove"));

var addOptions = """{"filename":"evidence.txt","mime":"text/plain","relationship":"data","deterministic":true}""";
var added = document.AssociatedFileAdd("bounded evidence"u8.ToArray(), addOptions);
File.WriteAllBytes("secure_mutation-associated.pdf", added.Bytes);
return 0;
