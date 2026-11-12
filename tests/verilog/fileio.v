// $fopen modes ("r" must not touch the file), file names from variables, $fdisplay/$fwrite,
// $readmemh of a file written by the same simulation.
module tb;
  integer fd, fr, i;
  reg [8*32-1:0] name;
  reg [7:0] mem [0:3];
  initial begin
    fr = $fopen("fileio_missing.txt", "r");
    $display("missing: %0d", fr);
    $sformat(name, "fileio_%0d.hex", 7);
    fd = $fopen(name, "w");
    $fdisplay(fd, "%h", 8'h12);
    $fwrite(fd, "%h\n%h\n", 8'h34, 8'h56);
    $fclose(fd);
    fr = $fopen(name, "r");
    $display("exists: %0d", fr != 0);
    $fclose(fr);
    fd = $fopen(name, "a");
    $fdisplay(fd, "78");
    $fclose(fd);
    $readmemh(name, mem);
    for (i = 0; i < 4; i = i + 1) $display("mem[%0d]=%h", i, mem[i]);
  end
endmodule
