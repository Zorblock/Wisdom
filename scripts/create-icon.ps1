param(
    [string]$Source = (Join-Path (Split-Path -Parent $PSScriptRoot) 'assets\logo.png'),
    [string]$Destination = (Join-Path (Split-Path -Parent $PSScriptRoot) 'assets\wisdom.ico')
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Source)) { throw "Logo not found: $Source" }

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.IO;

public static class WisdomIconWriter
{
    public static void Create(string input, string output)
    {
        int[] sizes = { 16, 20, 24, 32, 40, 48, 64, 96, 128, 256 };
        var frames = new List<byte[]>();
        using (var source = new Bitmap(input))
        {
            // The complete owl is too detailed at taskbar sizes. Keep the original
            // artwork for large assets, but use its bold head mark for the app icon.
            var cropSize = source.Width * 0.68f;
            var sourceCrop = new RectangleF(
                (source.Width - cropSize) / 2f,
                source.Height * 0.005f,
                cropSize,
                cropSize
            );
            foreach (var size in sizes)
            {
                using (var target = new Bitmap(size, size, PixelFormat.Format32bppArgb))
                using (var graphics = Graphics.FromImage(target))
                using (var memory = new MemoryStream())
                {
                    graphics.Clear(Color.Transparent);
                    graphics.CompositingMode = CompositingMode.SourceCopy;
                    graphics.CompositingQuality = CompositingQuality.HighQuality;
                    graphics.InterpolationMode = InterpolationMode.HighQualityBicubic;
                    graphics.PixelOffsetMode = PixelOffsetMode.HighQuality;
                    graphics.SmoothingMode = SmoothingMode.HighQuality;
                    graphics.DrawImage(
                        source,
                        new Rectangle(0, 0, size, size),
                        sourceCrop,
                        GraphicsUnit.Pixel
                    );
                    target.Save(memory, ImageFormat.Png);
                    frames.Add(memory.ToArray());
                }
            }
        }

        using (var stream = File.Create(output))
        using (var writer = new BinaryWriter(stream))
        {
            writer.Write((ushort)0);
            writer.Write((ushort)1);
            writer.Write((ushort)sizes.Length);
            var offset = 6 + 16 * sizes.Length;
            for (var i = 0; i < sizes.Length; i++)
            {
                writer.Write((byte)(sizes[i] == 256 ? 0 : sizes[i]));
                writer.Write((byte)(sizes[i] == 256 ? 0 : sizes[i]));
                writer.Write((byte)0);
                writer.Write((byte)0);
                writer.Write((ushort)1);
                writer.Write((ushort)32);
                writer.Write(frames[i].Length);
                writer.Write(offset);
                offset += frames[i].Length;
            }
            foreach (var frame in frames) { writer.Write(frame); }
        }
    }
}
'@

New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Destination) | Out-Null
[WisdomIconWriter]::Create($Source, $Destination)
Write-Output "Created $Destination"
