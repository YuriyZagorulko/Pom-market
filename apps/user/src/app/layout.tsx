import "./globals.css";
import { ToastProvider } from "@/components/toast";
import { SessionProvider } from "@/lib/session";

export const metadata = { title: "Pom-Market" };

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>
        <ToastProvider>
          <SessionProvider role="user">{children}</SessionProvider>
        </ToastProvider>
      </body>
    </html>
  );
}
