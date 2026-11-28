// exit: 1
// A generic interface port connects to an interface instance, not to a module instance.
module leaf; endmodule
module user(interface p);
endmodule
module t;
  leaf l();
  user u(l);
endmodule
